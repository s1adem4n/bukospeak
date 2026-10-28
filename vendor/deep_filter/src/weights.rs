//! The model file: the networks' weights by name, which `examples/convert.rs` writes from the
//! fork's ONNX networks.
//!
//! The file is a sequence of tensors, each its name's length as a byte, the name in UTF-8, its
//! number of values as a little-endian `u32`, and the values as little-endian `f32`s. Each tensor
//! keeps the layout it has in the ONNX networks.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the model file ends inside a tensor")]
    Truncated,
    #[error("the model file has a tensor whose name isn't UTF-8")]
    InvalidName,
    #[error("the model file has no {0}")]
    Missing(&'static str),
    #[error("the model file's {name} has {found} values, not {expected}")]
    Size { name: &'static str, expected: usize, found: usize },
    #[error("the model file has {0}, which the networks don't take")]
    Unused(String),
}

/// A model file's tensors, taken out one by one as the networks are built.
pub(crate) struct Weights<'a> {
    tensors: Vec<(&'a str, &'a [u8])>,
}

impl<'a> Weights<'a> {
    pub(crate) fn read(mut bytes: &'a [u8]) -> Result<Self, Error> {
        let mut tensors = Vec::new();
        while let Some((&length, rest)) = bytes.split_first() {
            let (name, rest) = rest.split_at_checked(usize::from(length)).ok_or(Error::Truncated)?;
            let name = std::str::from_utf8(name).map_err(|_| Error::InvalidName)?;
            let (count, rest) = rest.split_first_chunk::<4>().ok_or(Error::Truncated)?;
            let size = usize::try_from(u32::from_le_bytes(*count)).expect("a u32 fits a usize") * 4;
            let (values, rest) = rest.split_at_checked(size).ok_or(Error::Truncated)?;
            tensors.push((name, values));
            bytes = rest;
        }
        Ok(Self { tensors })
    }

    /// The tensor `name`, which has `len` values.
    pub(crate) fn take(&mut self, name: &'static str, len: usize) -> Result<Vec<f32>, Error> {
        let index = self.tensors.iter().position(|(tensor, _)| *tensor == name).ok_or(Error::Missing(name))?;
        let (_, values) = self.tensors.swap_remove(index);
        let found = values.len() / 4;
        if found != len {
            return Err(Error::Size { name, expected: len, found });
        }
        Ok(values.as_chunks::<4>().0.iter().map(|value| f32::from_le_bytes(*value)).collect())
    }

    /// Fails if a tensor is left that no network took.
    pub(crate) fn finish(self) -> Result<(), Error> {
        match self.tensors.first() {
            Some((name, _)) => Err(Error::Unused((*name).to_owned())),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tensor(name: &str, values: &[f32]) -> Vec<u8> {
        let mut bytes = vec![name.len() as u8];
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&(values.len() as u32).to_le_bytes());
        values.iter().for_each(|value| bytes.extend_from_slice(&value.to_le_bytes()));
        bytes
    }

    #[test]
    fn tensors_are_taken_by_name_and_size() {
        let bytes = [tensor("a", &[1., 2.]), tensor("b", &[3.])].concat();
        let mut weights = Weights::read(&bytes).unwrap();
        assert_eq!(weights.take("b", 1).unwrap(), [3.]);
        assert!(matches!(weights.take("a", 3), Err(Error::Size { name: "a", expected: 3, found: 2 })));
        assert!(matches!(weights.take("a", 2), Err(Error::Missing("a"))));
        weights.finish().unwrap();
    }

    #[test]
    fn a_tensor_left_or_cut_off_is_an_error() {
        let bytes = tensor("a", &[1., 2.]);
        assert!(matches!(Weights::read(&bytes[..bytes.len() - 1]), Err(Error::Truncated)));
        assert!(matches!(Weights::read(&bytes).unwrap().finish(), Err(Error::Unused(name)) if name == "a"));
    }
}
