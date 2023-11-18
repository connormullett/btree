use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
    path::PathBuf,
};

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

use crate::error::Error;

pub struct DataPage {
    values: Vec<String>,
    path: PathBuf,
}

impl DataPage {
    pub fn get(&self, idx: usize) -> Result<String, Error> {
        let value = self.values.get(idx).ok_or(Error::KeyNotFound)?;
        Ok(value.clone())
    }

    pub fn insert(&mut self, value: String) -> Result<usize, Error> {
        let idx = self.values.len();
        self.values.push(value);
        self.flush()?;
        Ok(idx)
    }

    pub fn delete(&mut self, idx: usize) -> Result<(), Error> {
        self.values.remove(idx);
        self.flush()?;
        Ok(())
    }

    pub fn flush(&self) -> Result<(), Error> {
        let mut fd = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&self.path)?;

        for value in self.values.iter() {
            let len = value.len();
            fd.write_u64::<BigEndian>(len as u64)?;
            fd.write_all(value.as_bytes())?
        }

        Ok(())
    }

    pub fn load(path: PathBuf) -> Result<Self, Error> {
        let mut fd = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path.clone())?;

        let mut values = vec![];

        loop {
            let len = match fd.read_u64::<BigEndian>() {
                Ok(len) => len,
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            };
            let mut bytes = vec![0u8; len as usize];
            fd.read_exact(&mut bytes)?;
            let value = std::str::from_utf8(&bytes).or(Err(Error::UTF8Error))?;
            values.push(value.to_string());
        }

        Ok(Self { values, path })
    }
}
