//! Optional lecture audio recording, synced to ink. Stored as Opus in an Ogg container.
//! Nothing here runs (no threads, no audio device) until `Recorder::start` is called.

use opus_pure::{Application, OggOpusReader, OggOpusWriter, OpusEncoder, OpusHead, Trim, MAX_PACKET_BYTES, MAX_PACKET_SAMPLES};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

const RATE: i32 = 48_000;

/// Linear-interpolating resample of mono f32 audio.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let n = (input.len() as u64 * to as u64 / from as u64) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * from as f64 / to as f64;
            let (a, f) = (pos.floor() as usize, (pos - pos.floor()) as f32);
            let x0 = input[a.min(input.len() - 1)];
            let x1 = input[(a + 1).min(input.len() - 1)];
            x0 + (x1 - x0) * f
        })
        .collect()
}

/// Encode mono 48 kHz PCM to Ogg Opus bytes (speech-tuned, 32 kbit/s).
pub fn encode_ogg_opus(pcm: &[f32]) -> Result<Vec<u8>, String> {
    let frame = (RATE / 50) as usize;
    let mut enc = OpusEncoder::new(RATE, 1, Application::Voip).map_err(|e| e.to_string())?;
    enc.bitrate_bps = 32_000;
    let head = OpusHead::for_encoder(&enc, RATE as u32);
    let ticks = 48_000 / RATE as usize;
    let total = pcm.len();
    let frames = (total + (head.pre_skip as usize).div_ceil(ticks)).div_ceil(frame).max(1);
    let final_granule = u64::from(head.pre_skip) + (total * ticks) as u64;
    let mut out = Vec::new();
    {
        let mut w = OggOpusWriter::new(&mut out, head).map_err(|e| e.to_string())?;
        let mut packet = vec![0u8; MAX_PACKET_BYTES];
        let mut block = vec![0.0f32; frame];
        for i in 0..frames {
            let start = (i * frame).min(total);
            let end = (start + frame).min(total);
            block[..end - start].copy_from_slice(&pcm[start..end]);
            block[end - start..].fill(0.0);
            let n = enc.encode(&block, frame, &mut packet).map_err(|e| e.to_string())?;
            if i + 1 == frames {
                let dur = final_granule - w.granule() as u64;
                w.write_packet_with_duration(&packet[..n], dur as u32).map_err(|e| e.to_string())?;
            } else {
                w.write_packet(&packet[..n]).map_err(|e| e.to_string())?;
            }
        }
        w.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// Decode Ogg Opus bytes to mono 48 kHz PCM (for playback).
pub fn decode_ogg_opus(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let mut r = OggOpusReader::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let head = r.head().clone();
    let ch = head.channel_count as usize;
    let mut dec = head.decoder(RATE).map_err(|e| e.to_string())?;
    let mut trim = Trim::new(&head, RATE, ch).map_err(|e| e.to_string())?;
    let mut block = vec![0.0f32; MAX_PACKET_SAMPLES * ch];
    let mut out = Vec::new();
    for p in r.packets() {
        let p = p.map_err(|e| e.to_string())?;
        let n = dec.decode(&p.data, MAX_PACKET_SAMPLES, &mut block).map_err(|e| e.to_string())?;
        let kept = trim.keep(&p, &block[..n * ch]);
        // downmix to mono
        out.extend(kept.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32));
    }
    Ok(out)
}

/// Which audio moment each stroke/block was written at: tap a stroke, play from there.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioSync {
    /// (stroke id, ms since recording started), ascending.
    pub marks: Vec<(u64, u32)>,
    pub duration_ms: u32,
}

impl AudioSync {
    pub fn mark(&mut self, stroke_id: u64, at_ms: u32) {
        self.marks.push((stroke_id, at_ms));
        self.duration_ms = self.duration_ms.max(at_ms);
    }
    /// Playback start for a stroke, a little before it was written for context.
    pub fn seek_ms(&self, stroke_id: u64) -> Option<u32> {
        self.marks.iter().find(|(id, _)| *id == stroke_id).map(|(_, t)| t.saturating_sub(1500))
    }
}

/// Microphone recorder. Visible "recording" state is the caller's job (`is_recording`).
pub struct Recorder {
    stream: Option<cpal::Stream>,
    buf: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    started: std::time::Instant,
}

impl Recorder {
    /// Opens the default input device and starts capturing. Off until called.
    pub fn start() -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let dev = host.default_input_device().ok_or("no microphone found")?;
        let cfg = dev.default_input_config().map_err(|e| e.to_string())?;
        let rate = cfg.sample_rate();
        let ch = cfg.channels() as usize;
        let buf = Arc::new(Mutex::new(Vec::<f32>::new()));
        let b = buf.clone();
        let err = |e| eprintln!("audio stream error: {e}");
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => dev.build_input_stream(
                cfg.config(),
                move |d: &[f32], _: &_| b.lock().unwrap().extend(d.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32)),
                err,
                None,
            ),
            cpal::SampleFormat::I16 => dev.build_input_stream(
                cfg.config(),
                move |d: &[i16], _: &_| b.lock().unwrap().extend(d.chunks(ch).map(|c| c.iter().map(|s| *s as f32 / 32768.0).sum::<f32>() / ch as f32)),
                err,
                None,
            ),
            other => return Err(format!("unsupported microphone format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { stream: Some(stream), buf, rate, started: std::time::Instant::now() })
    }

    pub fn is_recording(&self) -> bool {
        self.stream.is_some()
    }

    /// Milliseconds since the recording began (use for `AudioSync::mark`).
    pub fn elapsed_ms(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    /// Stop capturing and return the Ogg Opus bytes.
    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        self.stream.take(); // dropping the stream stops the device
        let pcm = std::mem::take(&mut *self.buf.lock().unwrap());
        encode_ogg_opus(&resample(&pcm, self.rate, RATE as u32))
    }
}

/// Play mono 48 kHz PCM from `start_ms` on the default output device. Dropping the stream stops it.
pub fn play(pcm: Vec<f32>, start_ms: u32) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let dev = host.default_output_device().ok_or("no audio output device")?;
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    if cfg.sample_format() != cpal::SampleFormat::F32 {
        return Err("output device does not support f32".into());
    }
    let (rate, ch) = (cfg.sample_rate(), cfg.channels() as usize);
    let data = resample(&pcm, RATE as u32, rate);
    let mut pos = (start_ms as u64 * rate as u64 / 1000) as usize;
    let stream = dev
        .build_output_stream(
            cfg.config(),
            move |out: &mut [f32], _: &_| {
                for frame in out.chunks_mut(ch) {
                    let s = data.get(pos).copied().unwrap_or(0.0);
                    pos += 1;
                    frame.fill(s);
                }
            },
            |e| eprintln!("audio output error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, secs: f32) -> Vec<f32> {
        (0..(48_000.0 * secs) as usize).map(|i| 0.4 * (2.0 * std::f32::consts::PI * hz * i as f32 / 48_000.0).sin()).collect()
    }

    #[test]
    fn opus_roundtrip_is_small_and_keeps_the_tone() {
        let pcm = sine(440.0, 2.0);
        let ogg = encode_ogg_opus(&pcm).unwrap();
        assert!(ogg.starts_with(b"OggS"));
        assert!(ogg.len() < 2 * 32_000 / 8 * 2, "2 s of speech-rate Opus stays tiny: {} bytes", ogg.len());
        let back = decode_ogg_opus(&ogg).unwrap();
        assert!((back.len() as i64 - pcm.len() as i64).abs() < 960, "gapless length: {} vs {}", back.len(), pcm.len());
        // energy of a 440 Hz tone survives
        let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
        let mid = &back[8000..back.len() - 8000];
        assert!(rms(mid) > 0.2 && rms(mid) < 0.4, "rms {}", rms(mid));
    }

    #[test]
    fn resampler_changes_length() {
        let a = vec![0.0f32; 44_100];
        assert_eq!(resample(&a, 44_100, 48_000).len(), 48_000);
        assert_eq!(resample(&a, 48_000, 48_000).len(), 44_100);
    }

    #[test]
    fn audio_sync_seeks_to_where_a_stroke_was_written() {
        let mut s = AudioSync::default();
        s.mark(11, 5_000);
        s.mark(12, 9_000);
        assert_eq!(s.seek_ms(12), Some(7_500));
        assert_eq!(s.seek_ms(11), Some(3_500));
        assert_eq!(s.seek_ms(99), None);
        assert_eq!(s.duration_ms, 9_000);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<AudioSync>(&json).unwrap(), s);
    }
}
