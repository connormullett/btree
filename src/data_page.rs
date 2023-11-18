use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::FileExt,
    path::PathBuf,
};

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

use crate::error::Error;

pub struct DataPage {
    file: File,
}

impl DataPage {
    pub fn new(page_id: usize) -> Result<Self, Error> {
        // TODO: use env var or something to set file location
        let path = PathBuf::from(format!("/tmp/{}", page_id.to_string()));
        let fd = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .append(true)
            .open(&path)?;

        let mut data_page = Self { file: fd };

        data_page.write_at_offset(0, page_id.to_be_bytes().to_vec())?;
        data_page.file.flush()?;

        Ok(data_page)
    }

    pub fn get_end(&mut self) -> Result<usize, Error> {
        let offset = self.file.seek(SeekFrom::End(0))?;
        Ok(offset as usize)
    }

    pub fn write_at_offset(&mut self, offset: usize, value: Vec<u8>) -> Result<(), Error> {
        let len_value = value.len();
        self.file.seek(SeekFrom::Start(offset as u64))?;
        self.file.write_u64::<BigEndian>(len_value as u64)?;
        self.file.write_all(&value)?;
        Ok(())
    }

    pub fn get_value_from_offset(&mut self, offset: usize) -> Result<Vec<u8>, Error> {
        // seek to offset
        self.file.seek(SeekFrom::Start(offset as u64))?;
        // get length of the actual value (sizeof usize)
        let len_value = self.file.read_u64::<BigEndian>().unwrap();
        // read actual value
        let mut value = Vec::with_capacity(len_value as usize);
        self.file.read_exact_at(&mut value, len_value)?;
        Ok(value)
    }
}
