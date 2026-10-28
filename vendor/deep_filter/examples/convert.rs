//! Writes the model file this crate reads from the fork's ONNX networks:
//! `convert export/ models/DeepFilterNet3.bin`, where `export/` holds `enc.onnx`,
//! `erb_dec.onnx` and `df_dec.onnx` as the fork's `DeepFilterNet3_onnx.tar.gz` unpacks them.
//!
//! Each weight is found as an input of the ONNX node that takes it, so by the node's name, and
//! written under the name the crate's networks take it by, in its ONNX layout. The ONNX files
//! are read with as much of protobuf's wire format as they need.

use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// A protobuf message's fields, in order: each its number and, for the wire types used here,
/// its varint or its bytes.
fn fields(mut bytes: &[u8]) -> Result<Vec<(u64, Field<'_>)>> {
    let mut fields = Vec::new();
    while !bytes.is_empty() {
        let key = varint(&mut bytes)?;
        let field = match key & 7 {
            0 => Field::Varint(varint(&mut bytes)?),
            1 => Field::Bytes(take(&mut bytes, 8)?),
            2 => {
                let len = usize::try_from(varint(&mut bytes)?)?;
                Field::Bytes(take(&mut bytes, len)?)
            }
            5 => Field::Bytes(take(&mut bytes, 4)?),
            wire => return Err(format!("protobuf wire type {wire} isn't supported").into()),
        };
        fields.push((key >> 3, field));
    }
    Ok(fields)
}

enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

impl<'a> Field<'a> {
    fn bytes(&self) -> Result<&'a [u8]> {
        match self {
            Field::Bytes(bytes) => Ok(bytes),
            Field::Varint(_) => Err("a protobuf field is a varint where bytes were expected".into()),
        }
    }

    fn string(&self) -> Result<&'a str> {
        Ok(std::str::from_utf8(self.bytes()?)?)
    }
}

fn varint(bytes: &mut &[u8]) -> Result<u64> {
    let mut value = 0;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = bytes.split_first().ok_or("a protobuf varint is cut off")?;
        *bytes = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("a protobuf varint is too long".into())
}

fn take<'a>(bytes: &mut &'a [u8], len: usize) -> Result<&'a [u8]> {
    let (taken, rest) = bytes.split_at_checked(len).ok_or("a protobuf field is cut off")?;
    *bytes = rest;
    Ok(taken)
}

/// A float tensor: its shape and its values.
struct Tensor {
    dims: Vec<usize>,
    values: Vec<f32>,
}

/// Reads an ONNX `TensorProto`: its name and the tensor, which must hold floats.
fn tensor(bytes: &[u8]) -> Result<(String, Tensor)> {
    const FLOAT: u64 = 1;
    let (mut name, mut dims, mut values, mut data_type) = (String::new(), Vec::new(), Vec::new(), None);
    for (number, field) in fields(bytes)? {
        match (number, field) {
            (1, Field::Varint(dim)) => dims.push(usize::try_from(dim)?),
            (1, Field::Bytes(mut packed)) => {
                while !packed.is_empty() {
                    dims.push(usize::try_from(varint(&mut packed)?)?);
                }
            }
            (2, Field::Varint(kind)) => data_type = Some(kind),
            (4 | 9, field) => values.extend(field.bytes()?.as_chunks::<4>().0.iter().map(|value| f32::from_le_bytes(*value))),
            (8, field) => name = field.string()?.to_owned(),
            _ => (),
        }
    }
    if data_type != Some(FLOAT) {
        return Err(format!("{name} isn't a float tensor").into());
    }
    Ok((name, Tensor { dims, values }))
}

/// An ONNX graph: its nodes' inputs by the nodes' names, and its constant tensors by name,
/// initializers and the outputs of `Constant` nodes alike.
struct Graph {
    inputs: HashMap<String, Vec<String>>,
    tensors: HashMap<String, Tensor>,
}

impl Graph {
    fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let model = fields(&bytes)?;
        let graph = model.iter().find(|(number, _)| *number == 7).ok_or("the ONNX model has no graph")?.1.bytes()?;
        let (mut inputs, mut tensors) = (HashMap::new(), HashMap::new());
        for (number, field) in fields(graph)? {
            match number {
                1 => {
                    let (mut name, mut op, mut node_inputs, mut outputs, mut value) = ("", "", Vec::new(), Vec::new(), None);
                    for (number, field) in fields(field.bytes()?)? {
                        match number {
                            1 => node_inputs.push(field.string()?.to_owned()),
                            2 => outputs.push(field.string()?.to_owned()),
                            3 => name = field.string()?,
                            4 => op = field.string()?,
                            5 => {
                                let attribute = fields(field.bytes()?)?;
                                if let Some((_, tensor)) = attribute.iter().find(|(number, _)| *number == 5) {
                                    value = Some(tensor.bytes()?);
                                }
                            }
                            _ => (),
                        }
                    }
                    if let ("Constant", Some(value), [output]) = (op, value, outputs.as_slice()) {
                        // A constant that isn't a float, a shape for instance, is no weight.
                        if let Ok((_, tensor)) = tensor(value) {
                            tensors.insert(output.clone(), tensor);
                        }
                    }
                    inputs.insert(name.to_owned(), node_inputs);
                }
                5 => {
                    let (name, tensor) = tensor(field.bytes()?)?;
                    tensors.insert(name, tensor);
                }
                _ => (),
            }
        }
        Ok(Self { inputs, tensors })
    }

    /// The weight that the node `node` takes as its input `index`, which has the shape `dims`.
    fn weight(&self, node: &str, index: usize, dims: &[usize]) -> Result<&[f32]> {
        let input = self.inputs.get(node).and_then(|inputs| inputs.get(index)).ok_or(format!("no input {index} of {node}"))?;
        let tensor = self.tensors.get(input).ok_or(format!("{node}'s input {input} isn't a constant"))?;
        if tensor.dims != dims {
            return Err(format!("{node}'s input {input} has the shape {:?}, not {dims:?}", tensor.dims).into());
        }
        Ok(&tensor.values)
    }
}

/// The model file being written.
struct Model {
    bytes: Vec<u8>,
}

impl Model {
    /// Writes `graph`'s weight as `name`.
    fn put(&mut self, name: &str, graph: &Graph, node: &str, index: usize, dims: &[usize]) -> Result<()> {
        let values = graph.weight(node, index, dims)?;
        self.bytes.push(u8::try_from(name.len())?);
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes.extend_from_slice(&u32::try_from(values.len())?.to_le_bytes());
        values.iter().for_each(|value| self.bytes.extend_from_slice(&value.to_le_bytes()));
        Ok(())
    }

    /// Writes a convolution's weight, and its bias if it has one.
    fn conv(&mut self, name: &str, graph: &Graph, node: &str, dims: [usize; 4], bias: Option<&str>) -> Result<()> {
        self.put(name, graph, node, 1, &dims)?;
        match bias {
            Some(bias) => self.put(bias, graph, node, 2, &dims[..1]),
            None => Ok(()),
        }
    }

    /// Writes a GRU's input and recurrent weights and its biases, as `{name}.input`,
    /// `{name}.recurrent` and `{name}.bias`.
    fn gru(&mut self, name: &str, graph: &Graph, node: &str) -> Result<()> {
        self.put(&format!("{name}.input"), graph, node, 1, &[1, 768, 256])?;
        self.put(&format!("{name}.recurrent"), graph, node, 2, &[1, 768, 256])?;
        self.put(&format!("{name}.bias"), graph, node, 3, &[1, 1536])
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [export, output] = args.as_slice() else {
        return Err("usage: convert EXPORT_DIRECTORY DeepFilterNet3.bin".into());
    };
    let export = Path::new(export);
    let mut model = Model { bytes: Vec::new() };
    let pointwise = [64, 64, 1, 1];
    let depthwise = [64, 1, 1, 3];

    let enc = Graph::read(&export.join("enc.onnx"))?;
    model.conv("enc.erb_conv0.weight", &enc, "/erb_conv0/1/Conv", [64, 1, 3, 3], Some("enc.erb_conv0.bias"))?;
    model.conv("enc.erb_conv1.depthwise", &enc, "/erb_conv1/0/Conv", depthwise, None)?;
    model.conv("enc.erb_conv1.pointwise", &enc, "/erb_conv1/1/Conv", pointwise, Some("enc.erb_conv1.bias"))?;
    model.conv("enc.erb_conv2.depthwise", &enc, "/erb_conv2/Conv", depthwise, None)?;
    model.conv("enc.erb_conv2.pointwise", &enc, "/erb_conv2/Conv_1", pointwise, Some("enc.erb_conv2.bias"))?;
    model.conv("enc.erb_conv3.depthwise", &enc, "/erb_conv3/0/Conv", depthwise, None)?;
    model.conv("enc.erb_conv3.pointwise", &enc, "/erb_conv3/1/Conv", pointwise, Some("enc.erb_conv3.bias"))?;
    model.conv("enc.df_conv0.weight", &enc, "/df_conv0/1/Conv", [64, 1, 3, 3], None)?;
    model.conv("enc.df_conv0.pointwise", &enc, "/df_conv0/2/Conv", pointwise, Some("enc.df_conv0.bias"))?;
    model.conv("enc.df_conv1.depthwise", &enc, "/df_conv1/Conv", depthwise, None)?;
    model.conv("enc.df_conv1.pointwise", &enc, "/df_conv1/Conv_1", pointwise, Some("enc.df_conv1.bias"))?;
    model.put("enc.df_fc_emb", &enc, "/df_fc_emb/0/Einsum", 1, &[32, 96, 16])?;
    model.put("enc.emb_gru.linear_in", &enc, "/emb_gru/linear_in/0/Einsum", 1, &[16, 32, 16])?;
    model.gru("enc.emb_gru.gru", &enc, "/emb_gru/GRU")?;
    model.put("enc.emb_gru.linear_out", &enc, "/emb_gru/linear_out/0/Einsum", 1, &[16, 16, 32])?;
    model.put("enc.lsnr_fc.weight", &enc, "/lsnr_fc/0/MatMul", 1, &[512, 1])?;
    model.put("enc.lsnr_fc.bias", &enc, "/lsnr_fc/0/Add", 0, &[1])?;

    let erb_dec = Graph::read(&export.join("erb_dec.onnx"))?;
    model.put("erb_dec.emb_gru.linear_in", &erb_dec, "/emb_gru/linear_in/Einsum", 1, &[16, 32, 16])?;
    model.gru("erb_dec.emb_gru.gru0", &erb_dec, "/emb_gru/GRU")?;
    model.gru("erb_dec.emb_gru.gru1", &erb_dec, "/emb_gru/GRU_1")?;
    model.put("erb_dec.emb_gru.linear_out", &erb_dec, "/emb_gru/linear_out/Einsum", 1, &[16, 16, 32])?;
    model.conv("erb_dec.conv3p.weight", &erb_dec, "/conv3p/0/Conv", [64, 1, 1, 1], Some("erb_dec.conv3p.bias"))?;
    model.conv("erb_dec.convt3.depthwise", &erb_dec, "/convt3/Conv", depthwise, None)?;
    model.conv("erb_dec.convt3.pointwise", &erb_dec, "/convt3/Conv_1", pointwise, Some("erb_dec.convt3.bias"))?;
    model.conv("erb_dec.conv2p.weight", &erb_dec, "/conv2p/Conv", [64, 1, 1, 1], Some("erb_dec.conv2p.bias"))?;
    model.conv("erb_dec.convt2.depthwise", &erb_dec, "/convt2/0/ConvTranspose", depthwise, None)?;
    model.conv("erb_dec.convt2.pointwise", &erb_dec, "/convt2/1/Conv", pointwise, Some("erb_dec.convt2.bias"))?;
    model.conv("erb_dec.conv1p.weight", &erb_dec, "/conv1p/Conv", [64, 1, 1, 1], Some("erb_dec.conv1p.bias"))?;
    model.conv("erb_dec.convt1.depthwise", &erb_dec, "/convt1/ConvTranspose", depthwise, None)?;
    model.conv("erb_dec.convt1.pointwise", &erb_dec, "/convt1/Conv", pointwise, Some("erb_dec.convt1.bias"))?;
    model.conv("erb_dec.conv0p.weight", &erb_dec, "/conv0p/Conv", [64, 1, 1, 1], Some("erb_dec.conv0p.bias"))?;
    model.conv("erb_dec.conv0_out.weight", &erb_dec, "/conv0_out/0/Conv", [1, 64, 1, 3], Some("erb_dec.conv0_out.bias"))?;

    let df_dec = Graph::read(&export.join("df_dec.onnx"))?;
    model.put("df_dec.df_gru.linear_in", &df_dec, "/df_gru/linear_in/linear_in.0/Einsum", 1, &[8, 64, 32])?;
    model.gru("df_dec.df_gru.gru0", &df_dec, "/df_gru/gru/GRU")?;
    model.gru("df_dec.df_gru.gru1", &df_dec, "/df_gru/gru/GRU_1")?;
    model.put("df_dec.df_skip", &df_dec, "/df_skip/Einsum", 1, &[16, 32, 16])?;
    model.put("df_dec.df_out", &df_dec, "/df_out/df_out.0/Einsum", 1, &[16, 16, 60])?;
    model.conv("df_dec.df_convp.weight", &df_dec, "/df_convp/df_convp.1/Conv", [10, 32, 5, 1], None)?;
    model.conv("df_dec.df_convp.pointwise", &df_dec, "/df_convp/df_convp.2/Conv", [10, 10, 1, 1], Some("df_dec.df_convp.bias"))?;

    std::fs::write(output, &model.bytes)?;
    Ok(())
}
