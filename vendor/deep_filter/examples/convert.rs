//! Converts the fork's ONNX model archive into the one this crate reads:
//! `convert DeepFilterNet3_onnx.tar.gz DeepFilterNet3.tar.gz`.
//!
//! Each network is pulsed to take a hop at a time, as the fork does on every start, and written
//! as NNEF beside the archive's `config.ini`. So the crate loads networks ready to optimize, and
//! needs neither tract's ONNX importer nor its pulsing.

use std::error::Error;
use std::fs::File;
use std::io::{Cursor, Read};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use ini::Ini;
use tar::{Archive, Builder, Header};
use tract_onnx::prelude::*;
use tract_onnx::tract_hir::shapefactoid;
use tract_pulse::internal::ToDim;
use tract_pulse::model::{PulsedModel, PulsedModelExt};
use tract_pulse::WithPulse;

/// The outputs a network keeps: those of the ONNX network before pulsing, or of the pulsed one.
enum Outputs {
    Before(&'static [&'static str]),
    Pulsed(&'static [&'static str]),
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        return Err("usage: convert DeepFilterNet3_onnx.tar.gz DeepFilterNet3.tar.gz".into());
    };
    let mut members = Vec::new();
    let mut archive = Archive::new(GzDecoder::new(File::open(input)?));
    for entry in archive.entries()? {
        let mut file = entry?;
        let name = file.path()?.file_name().and_then(|name| name.to_str()).map(str::to_owned);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if let Some(name) = name {
            members.push((name, bytes));
        }
    }
    let member = |name: &str| {
        members.iter().find(|(member, _)| member == name).map(|(_, bytes)| bytes.as_slice()).ok_or(format!("{input} has no {name}"))
    };
    let config_bytes = member("config.ini")?;
    let config = Ini::read_from(&mut Cursor::new(config_bytes))?;
    let get = |section: &str, key: &str| -> Result<usize, Box<dyn Error>> {
        Ok(config.section(Some(section)).and_then(|properties| properties.get(key)).ok_or(format!("no {key} in [{section}]"))?.parse()?)
    };
    let (nb_erb, nb_df, layer_width) = (get("df", "nb_erb")?, get("df", "nb_df")?, get("deepfilternet", "conv_ch")?);
    let (n_hidden, e1f, e3f) = (layer_width * nb_erb / 4, nb_erb / 2, nb_erb / 4);
    let fact = |shape| InferenceFact::dt_shape(f32::datum_type(), shape);

    let enc = pulsed(
        member("enc.onnx")?,
        |s| vec![("feat_erb", fact(shapefactoid!(1, 1, s, nb_erb))), ("feat_spec", fact(shapefactoid!(1, 2, s, nb_df)))],
        Outputs::Before(&["e0", "e1", "e2", "e3", "emb", "c0", "lsnr"]),
    )?;
    let erb_dec = pulsed(
        member("erb_dec.onnx")?,
        |s| {
            vec![
                ("emb", fact(shapefactoid!(1, s, n_hidden))),
                ("e3", fact(shapefactoid!(1, layer_width, s, e3f))),
                ("e2", fact(shapefactoid!(1, layer_width, s, e3f))),
                ("e1", fact(shapefactoid!(1, layer_width, s, e1f))),
                ("e0", fact(shapefactoid!(1, layer_width, s, nb_erb))),
            ]
        },
        Outputs::Pulsed(&["m"]),
    )?;
    let df_dec = pulsed(
        member("df_dec.onnx")?,
        |s| vec![("emb", fact(shapefactoid!(1, s, n_hidden))), ("c0", fact(shapefactoid!(1, layer_width, s, nb_df)))],
        Outputs::Before(&["coefs"]),
    )?;

    let nnef = tract_nnef::nnef().with_pulse();
    let mut builder = Builder::new(GzEncoder::new(File::create(output)?, flate2::Compression::best()));
    let mut add = |name: &str, bytes: &[u8]| {
        let mut header = Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, name, bytes)
    };
    add("config.ini", config_bytes)?;
    for (name, network) in [("enc.nnef.tar", enc), ("erb_dec.nnef.tar", erb_dec), ("df_dec.nnef.tar", df_dec)] {
        add(name, &nnef.write_to_tar(&network, Vec::new())?)?;
    }
    builder.into_inner()?.finish()?;
    Ok(())
}

/// A network for its ONNX bytes, with the inputs `inputs` declares along its time axis `S`,
/// pulsed to take one hop at a time, and decluttered, which NNEF can express; optimizing is left
/// to the crate.
fn pulsed(onnx: &[u8], inputs: impl FnOnce(&Symbol) -> Vec<(&'static str, InferenceFact)>, outputs: Outputs) -> TractResult<TypedModel> {
    let mut m = tract_onnx::onnx().with_ignore_output_shapes(true).model_for_read(&mut Cursor::new(onnx))?;
    let s = m.symbols.sym("S");
    let inputs = inputs(&s);
    for (index, (_, fact)) in inputs.iter().enumerate() {
        m = m.with_input_fact(index, fact.clone())?;
    }
    m = m.with_input_names(inputs.iter().map(|(name, _)| *name))?;
    if let Outputs::Before(names) = outputs {
        m = m.with_outputs_by_name(names)?;
    }
    m.analyse(true)?;
    let mut m = m.into_typed()?;
    m.declutter()?;
    let mut m = PulsedModel::new(&m, s, &1.to_dim())?.into_typed()?;
    if let Outputs::Pulsed(names) = outputs {
        m = m.with_outputs_by_name(names)?;
    }
    m.declutter()?;
    Ok(m)
}
