//! Write the trained network as an ONNX file using the protobuf types bundled with tract.

use crate::model::Net;
use burn::prelude::*;
use tract_onnx::pb::{
    attribute_proto::AttributeType, tensor_proto::DataType, tensor_shape_proto, tensor_shape_proto::Dimension,
    type_proto, AttributeProto, GraphProto, ModelProto, NodeProto, OperatorSetIdProto, TensorProto, TensorShapeProto,
    TypeProto, ValueInfoProto,
};

fn floats<const D: usize>(t: Tensor<D>) -> (Vec<i64>, Vec<f32>) {
    let data = t.into_data();
    (data.shape().iter().map(|&d| d as i64).collect(), data.try_to_vec::<f32>().expect("f32 weights"))
}

fn tensor(name: &str, dims: Vec<i64>, data: Vec<f32>) -> TensorProto {
    TensorProto { name: name.into(), dims, data_type: DataType::Float as i32, float_data: data, ..Default::default() }
}

fn ints(name: &str, v: &[i64]) -> AttributeProto {
    AttributeProto { name: name.into(), r#type: AttributeType::Ints as i32, ints: v.to_vec(), ..Default::default() }
}
fn int(name: &str, v: i64) -> AttributeProto {
    AttributeProto { name: name.into(), r#type: AttributeType::Int as i32, i: v, ..Default::default() }
}

fn node(op: &str, name: &str, inputs: &[&str], out: &str, attrs: Vec<AttributeProto>) -> NodeProto {
    NodeProto {
        op_type: op.into(),
        name: name.into(),
        input: inputs.iter().map(|s| s.to_string()).collect(),
        output: vec![out.into()],
        attribute: attrs,
        ..Default::default()
    }
}

fn value_info(name: &str, dims: &[i64]) -> ValueInfoProto {
    let dim = dims
        .iter()
        .map(|&d| Dimension { value: Some(tensor_shape_proto::dimension::Value::DimValue(d)), ..Default::default() })
        .collect();
    ValueInfoProto {
        name: name.into(),
        r#type: Some(TypeProto {
            value: Some(type_proto::Value::TensorType(type_proto::Tensor {
                elem_type: DataType::Float as i32,
                shape: Some(TensorShapeProto { dim }),
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn to_onnx(net: &Net, classes: usize) -> Vec<u8> {
    use prost::Message;
    let mut inits = Vec::new();
    let mut nodes = Vec::new();
    let mut cur = "x".to_string();
    let mut add_conv = |i: usize, c: &burn::nn::conv::Conv2d, pool: bool, cur: &mut String| {
        let (wd, w) = floats(c.weight.val());
        let (bd, b) = floats(c.bias.as_ref().expect("bias").val());
        inits.push(tensor(&format!("c{i}.w"), wd, w));
        inits.push(tensor(&format!("c{i}.b"), bd, b));
        let o = format!("c{i}.out");
        nodes.push(node(
            "Conv",
            &format!("conv{i}"),
            &[cur, &format!("c{i}.w"), &format!("c{i}.b")],
            &o,
            vec![ints("kernel_shape", &[3, 3]), ints("pads", &[1, 1, 1, 1]), ints("strides", &[1, 1])],
        ));
        let r = format!("c{i}.relu");
        nodes.push(node("Relu", &format!("relu{i}"), &[&o], &r, vec![]));
        *cur = r;
        if pool {
            let p = format!("c{i}.pool");
            nodes.push(node(
                "MaxPool",
                &format!("pool{i}"),
                &[cur],
                &p,
                vec![ints("kernel_shape", &[2, 2]), ints("strides", &[2, 2])],
            ));
            *cur = p;
        }
    };
    add_conv(1, &net.c1, false, &mut cur);
    add_conv(2, &net.c2, true, &mut cur);
    add_conv(3, &net.c3, false, &mut cur);
    add_conv(4, &net.c4, true, &mut cur);
    add_conv(5, &net.c5, true, &mut cur);
    nodes.push(node("Flatten", "flatten", &[&cur], "flat", vec![int("axis", 1)]));
    for (i, (l, relu)) in [(&net.fc1, true), (&net.fc2, false)].into_iter().enumerate() {
        let (wd, w) = floats(l.weight.val());
        let (bd, b) = floats(l.bias.as_ref().expect("bias").val());
        inits.push(tensor(&format!("fc{i}.w"), wd, w));
        inits.push(tensor(&format!("fc{i}.b"), bd, b));
        let input = if i == 0 { "flat".to_string() } else { "fc0.relu".to_string() };
        let out = if relu { format!("fc{i}.out") } else { "logits".to_string() };
        // burn Linear stores weight as [in, out], i.e. Gemm without transB.
        nodes.push(node("Gemm", &format!("gemm{i}"), &[&input, &format!("fc{i}.w"), &format!("fc{i}.b")], &out, vec![]));
        if relu {
            nodes.push(node("Relu", &format!("fcrelu{i}"), &[&out], &format!("fc{i}.relu"), vec![]));
        }
    }
    let graph = GraphProto {
        name: "inkwell_symbols".into(),
        node: nodes,
        initializer: inits,
        input: vec![value_info("x", &[1, 1, 32, 32])],
        output: vec![value_info("logits", &[1, classes as i64])],
        ..Default::default()
    };
    ModelProto {
        ir_version: 8,
        opset_import: vec![OperatorSetIdProto { domain: String::new(), version: 13 }],
        producer_name: "inkwell-train".into(),
        graph: Some(graph),
        ..Default::default()
    }
    .encode_to_vec()
}
