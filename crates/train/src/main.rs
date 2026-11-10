//! Offline trainer for the Inkwell symbol CNN. Pure Rust (burn). Runs on any GPU through
//! wgpu (Vulkan/DX12/Metal: AMD, NVIDIA, Intel incl. integrated) or on the CPU, and writes
//! an ONNX file that the app runs with tract.
//!
//!   inkwell-train download
//!   inkwell-train train [--device auto|gpu|cpu] [--epochs N] [--out DIR]

mod data;
mod export;
mod imgops;
mod model;
mod rng;
mod symbols;
mod synth;

use anyhow::{bail, Context, Result};
use burn::nn::loss::CrossEntropyLossConfig;
use burn::optim::{AdamWConfig, GradientsParams};
use burn::prelude::*;
use data::Set;
use imgops::SIZE;
use model::Net;
use rng::Rng;
use std::path::{Path, PathBuf};

const HASY_URL: &str = "https://zenodo.org/records/259444/files/HASYv2.tar.bz2?download=1";
const HASY_MD5: &str = "fddf23f36e24b5236f6b3a0880c778e3";

fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var("INKWELL_DATA").unwrap_or_else(|_| "training/data".into()))
}

fn download() -> Result<()> {
    use md5::{Digest, Md5};
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    let arc = dir.join("HASYv2.tar.bz2");
    if !arc.exists() {
        println!("downloading {HASY_URL}");
        let mut resp = ureq::get(HASY_URL).call()?;
        let mut f = std::fs::File::create(&arc)?;
        std::io::copy(&mut resp.body_mut().as_reader(), &mut f)?;
    }
    let bytes = std::fs::read(&arc)?;
    let sum: String = Md5::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if sum != HASY_MD5 {
        bail!("checksum mismatch: {sum} != {HASY_MD5}");
    }
    let out = dir.join("hasy");
    if !out.join("hasy-data").is_dir() {
        let mut ar = tar::Archive::new(bzip2::read::BzDecoder::new(&bytes[..]));
        ar.unpack(&out)?;
    }
    println!("ok: {out:?}");
    Ok(())
}

struct Args {
    device: String,
    epochs: usize,
    out: PathBuf,
}

fn parse_args(a: &[String]) -> Args {
    let mut r = Args { device: "auto".into(), epochs: 20, out: "training/out".into() };
    let mut it = a.iter();
    while let Some(k) = it.next() {
        match k.as_str() {
            "--device" => r.device = it.next().cloned().unwrap_or_default(),
            "--epochs" => r.epochs = it.next().and_then(|v| v.parse().ok()).unwrap_or(r.epochs),
            "--out" => r.out = it.next().map(PathBuf::from).unwrap_or(r.out.clone()),
            _ => {}
        }
    }
    r
}

fn batch(xs: &[&Vec<f32>], ys: &[usize], d: &Device) -> (Tensor<4>, Tensor<1, Int>) {
    let flat: Vec<f32> = xs.iter().flat_map(|v| v.iter().copied()).collect();
    let x = Tensor::<4>::from_data(TensorData::new(flat, [xs.len(), 1, SIZE, SIZE]), d);
    let y = Tensor::<1, Int>::from_data(TensorData::new(ys.iter().map(|&v| v as i64).collect::<Vec<_>>(), [ys.len()]), d);
    (x, y)
}

fn evaluate(net: &Net, set: &Set, d: &Device) -> (f32, f32) {
    let (mut c1, mut c5) = (0usize, 0usize);
    for chunk in (0..set.x.len()).collect::<Vec<_>>().chunks(512) {
        let xs: Vec<&Vec<f32>> = chunk.iter().map(|&i| &set.x[i]).collect();
        let ys: Vec<usize> = chunk.iter().map(|&i| set.y[i]).collect();
        let (x, _) = batch(&xs, &ys, d);
        let logits = net.forward(x).into_data().try_to_vec::<f32>().unwrap();
        let n = logits.len() / xs.len();
        for (row, &y) in logits.chunks(n).zip(&ys) {
            let mut idx: Vec<usize> = (0..n).collect();
            idx.sort_by(|&a, &b| row[b].total_cmp(&row[a]));
            c1 += (idx[0] == y) as usize;
            c5 += idx[..5].contains(&y) as usize;
        }
    }
    (c1 as f32 / set.x.len() as f32, c5 as f32 / set.x.len() as f32)
}

fn run(plain: Device, args: &Args) -> Result<()> {
    let device = plain.clone().autodiff();
    let classes = symbols::load();
    let root = data_dir().join("hasy");
    println!("loading data ...");
    let (train, test, synth_test) = data::build(&root, &classes)?;
    println!("classes {} | train {} | hasy test {} | synthetic test {}", classes.len(), train.x.len(), test.x.len(), synth_test.x.len());
    let mut net = Net::new(classes.len(), &device);
    let mut opt = AdamWConfig::new().with_weight_decay(1e-4).init();
    let loss_fn = CrossEntropyLossConfig::new().with_smoothing(Some(0.05)).init(&device);
    let bs = 128;
    let steps_per_epoch = train.x.len().div_ceil(bs);
    let total = (args.epochs * steps_per_epoch) as f64;
    let mut rng = Rng::new(1234);
    let mut step = 0f64;
    let t0 = std::time::Instant::now();
    for ep in 0..args.epochs {
        let mut order: Vec<usize> = (0..train.x.len()).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }
        let mut loss_sum = 0.0f64;
        for chunk in order.chunks(bs) {
            let aug: Vec<Vec<f32>> = chunk.iter().map(|&i| imgops::augment(&train.x[i], &mut rng)).collect();
            let xs: Vec<&Vec<f32>> = aug.iter().collect();
            let ys: Vec<usize> = chunk.iter().map(|&i| train.y[i]).collect();
            let (x, y) = batch(&xs, &ys, &device);
            let loss = loss_fn.forward(net.forward(x), y);
            loss_sum += loss.clone().into_scalar::<f32>() as f64 * chunk.len() as f64;
            let grads = GradientsParams::from_grads(loss.backward(), &net);
            // linear warmup then cosine decay
            let lr = 2e-3 * (step / 200.0).min(1.0) * (0.5 * (1.0 + (std::f64::consts::PI * step / total).cos())).max(0.02);
            net = opt.step(lr, net, grads);
            step += 1.0;
        }
        let (acc, _) = evaluate(&net.valid(), &test, &plain);
        println!("epoch {:2} loss {:.3} hasy-test acc {:.4} ({:.0}s)", ep + 1, loss_sum / train.x.len() as f64, acc, t0.elapsed().as_secs_f32());
    }
    let net = net.valid();
    let (t1, t5) = evaluate(&net, &test, &plain);
    let (s1, s5) = evaluate(&net, &synth_test, &plain);
    std::fs::create_dir_all(&args.out)?;
    let metrics = serde_json::json!({
        "hasy_fold1_test": {"n": test.x.len(), "top1": t1, "top5": t5},
        "synthetic_only_classes": {"n": synth_test.x.len(), "top1": s1, "top5": s5},
        "classes": classes.len(), "epochs": args.epochs,
    });
    std::fs::write(args.out.join("metrics.json"), serde_json::to_string_pretty(&metrics)?)?;
    let labels: Vec<&str> = classes.iter().map(|c| c.label.as_str()).collect();
    std::fs::write(args.out.join("labels.json"), serde_json::to_string(&labels)?)?;
    let onnx = export::to_onnx(&net, classes.len());
    std::fs::write(args.out.join("symbols.onnx"), &onnx)?;
    println!("{metrics:#}\nexported {:.2} MB", onnx.len() as f32 / 1e6);
    verify_export(&args.out.join("symbols.onnx"), &net, &test, &plain)
}

/// The exported ONNX must reproduce burn's logits when run in tract.
fn verify_export(path: &Path, net: &Net, test: &Set, d: &Device) -> Result<()> {
    use tract_onnx::prelude::*;
    let model = tract_onnx::onnx().model_for_path(path)?.into_optimized()?.into_runnable()?;
    let mut worst = 0f32;
    for i in (0..test.x.len()).step_by(test.x.len() / 20 + 1) {
        let (x, _) = batch(&[&test.x[i]], &[0], d);
        let want = net.forward(x).into_data().try_to_vec::<f32>().unwrap();
        let inp = tract_ndarray::Array4::from_shape_vec((1, 1, SIZE, SIZE), test.x[i].clone())?;
        let out = model.run(tvec!(Tensor::from(inp).into()))?;
        let got: Vec<f32> = out[0].to_plain_array_view::<f32>()?.iter().copied().collect();
        for (a, b) in want.iter().zip(&got) {
            worst = worst.max((a - b).abs());
        }
    }
    println!("ONNX vs burn max |logit diff| = {worst:.2e}");
    if worst > 1e-2 {
        bail!("exported ONNX disagrees with the trained model");
    }
    Ok(())
}

fn gpu_device() -> Option<Device> {
    std::panic::catch_unwind(|| {
        let d = Device::wgpu(Default::default());
        let t = Tensor::<1>::from_floats([1.0, 2.0], &d);
        (t.sum().into_scalar::<f32>() == 3.0).then_some(d)
    })
    .ok()
    .flatten()
}

fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match argv.first().map(String::as_str) {
        Some("download") => download(),
        Some("train") => {
            let args = parse_args(&argv[1..]);
            let dev = match args.device.as_str() {
                "cpu" => None,
                "gpu" => Some(gpu_device().context("no usable GPU (wgpu) found")?),
                _ => gpu_device(),
            };
            match dev {
                Some(d) => {
                    println!("backend: wgpu (GPU)");
                    run(d, &args)
                }
                None => {
                    println!("backend: flex (CPU)");
                    run(Device::flex(), &args)
                }
            }
        }
        _ => bail!("usage: inkwell-train download | train [--device auto|gpu|cpu] [--epochs N] [--out DIR]"),
    }
}
