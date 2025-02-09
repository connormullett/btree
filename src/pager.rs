use crate::error::Error;
use crate::node_type::Offset;
use crate::page::Page;
use crate::page_layout::PAGE_SIZE;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub struct Pager {
    file: File,
    cursor: usize,
}

impl Pager {
    pub fn new(path: &Path) -> Result<Pager, Error> {
        let fd = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(path)?;

        Ok(Pager {
            file: fd,
            cursor: 0,
        })
    }

    pub fn get_page(&mut self, offset: &Offset) -> Result<Page, Error> {
        let mut page: [u8; PAGE_SIZE] = [0x00; PAGE_SIZE];
        self.file.seek(SeekFrom::Start(offset.0 as u64))?;
        self.file.read_exact(&mut page)?;
        Ok(Page::new(page))
    }

    pub fn write_page(&mut self, page: Page) -> Result<Offset, Error> {
        self.file.seek(SeekFrom::Start(self.cursor as u64))?;
        self.file.write_all(&page.get_data())?;
        let res = Offset(self.cursor);
        self.cursor += PAGE_SIZE;
        Ok(res)
    }

    pub fn write_page_at_offset(&mut self, page: Page, offset: &Offset) -> Result<(), Error> {
        self.file.seek(SeekFrom::Start(offset.0 as u64))?;
        self.file.write_all(&page.get_data())?;
        Ok(())
    }
}
