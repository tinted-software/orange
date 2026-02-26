#![no_std]

#[cfg(not(target_os = "none"))]
extern crate std;

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::fmt;
use core::marker::PhantomData;
use spin::Mutex;
use unicode_normalization::UnicodeNormalization;
use zlib_rs::ReturnCode;
use zlib_rs::c_api::z_stream;
use zlib_rs::inflate::{InflateConfig, InflateStream, end, inflate, init, reset};

mod hfs_strings;
pub mod internal;

pub use crate::internal::*;
use hfs_strings::fast_unicode_compare;

pub enum SeekFrom {
    Start(u64),
    Current(i64),
    End(i64),
}

#[derive(Debug)]
pub enum Error {
    InvalidData(String),
    BadNode,
    InvalidRecordKey,
    InvalidRecordType,
    UnsupportedOperation,
    KeyNotFound,
}

pub type Result<T> = core::result::Result<T, Error>;

pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.read(buf) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = buf;
                    buf = &mut tmp[n..];
                }
                Err(e) => return Err(e),
            }
        }
        if !buf.is_empty() {
            Err(Error::InvalidData(String::from("Unexpected EOF")))
        } else {
            Ok(())
        }
    }
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
    fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf) {
                Ok(0) => break,
                Ok(n) => buf = &buf[n..],
                Err(e) => return Err(e),
            }
        }
        if !buf.is_empty() {
            Err(Error::InvalidData(String::from("Failed to write all data")))
        } else {
            Ok(())
        }
    }
}

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
}

pub trait ReadExt: Read {
    fn read_u16_be(&mut self) -> Result<u16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(u16::from_be_bytes(buf))
    }
    fn read_u32_be(&mut self) -> Result<u32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(u32::from_be_bytes(buf))
    }
    fn read_u64_be(&mut self) -> Result<u64> {
        let mut buf = [0u8; 8];
        self.read_exact(&mut buf)?;
        Ok(u64::from_be_bytes(buf))
    }
    fn read_i16_be(&mut self) -> Result<i16> {
        let mut buf = [0u8; 2];
        self.read_exact(&mut buf)?;
        Ok(i16::from_be_bytes(buf))
    }
    fn read_i32_be(&mut self) -> Result<i32> {
        let mut buf = [0u8; 4];
        self.read_exact(&mut buf)?;
        Ok(i32::from_be_bytes(buf))
    }
    fn read_u8(&mut self) -> Result<u8> {
        let mut buf = [0u8; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }
    fn read_i8(&mut self) -> Result<i8> {
        let mut buf = [0u8; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0] as i8)
    }
}

impl<T: Read + ?Sized> ReadExt for T {}

pub trait WriteExt: Write {
    fn write_u16_be(&mut self, n: u16) -> Result<()> {
        self.write_all(&n.to_be_bytes())
    }
    fn write_u32_be(&mut self, n: u32) -> Result<()> {
        self.write_all(&n.to_be_bytes())
    }
    fn write_u64_be(&mut self, n: u64) -> Result<()> {
        self.write_all(&n.to_be_bytes())
    }
    fn write_i8(&mut self, n: i8) -> Result<()> {
        self.write_all(&[n as u8])
    }
    fn write_u8(&mut self, n: u8) -> Result<()> {
        self.write_all(&[n])
    }
}

impl<T: Write + ?Sized> WriteExt for T {}

pub struct Cursor<T> {
    inner: T,
    pos: u64,
}

impl<T: AsRef<[u8]>> Cursor<T> {
    pub fn new(inner: T) -> Self {
        Self { inner, pos: 0 }
    }
}

impl<T: AsRef<[u8]>> Read for Cursor<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let inner = self.inner.as_ref();
        if self.pos >= inner.len() as u64 {
            return Ok(0);
        }
        let n = core::cmp::min(buf.len(), (inner.len() as u64 - self.pos) as usize);
        buf[..n].copy_from_slice(&inner[self.pos as usize..self.pos as usize + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl<T: AsRef<[u8]>> Seek for Cursor<T> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let inner = self.inner.as_ref();
        let new_pos = match pos {
            SeekFrom::Start(s) => s as i64,
            SeekFrom::Current(c) => self.pos as i64 + c,
            SeekFrom::End(e) => inner.len() as i64 + e,
        };
        if new_pos < 0 {
            return Err(Error::InvalidData(String::from("Invalid seek")));
        }
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct HFSString(pub Vec<u16>);

impl fmt::Debug for HFSString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for &c in &self.0 {
            if let Some(ch) = core::char::from_u32(c as u32) {
                write!(f, "{}", ch)?;
            } else {
                write!(f, "\\u{{{:04X}}}", c)?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for HFSString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for &c in &self.0 {
            if let Some(ch) = core::char::from_u32(c as u32) {
                write!(f, "{}", ch)?;
            }
        }
        Ok(())
    }
}

impl PartialOrd for HFSString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HFSString {
    fn cmp(&self, other: &Self) -> Ordering {
        fast_unicode_compare(&self.0[..], &other.0[..])
    }
}

pub trait HFSStringTrait:
    fmt::Debug + fmt::Display + Ord + PartialOrd + Eq + PartialEq + Clone + Sized
{
    fn from_vec(v: Vec<u16>) -> Self;

    fn to_vec(&self) -> Vec<u16>;
}

impl HFSStringTrait for HFSString {
    fn from_vec(v: Vec<u16>) -> Self {
        HFSString(v)
    }

    fn to_vec(&self) -> Vec<u16> {
        self.0.clone()
    }
}

#[derive(Clone, PartialEq, Eq)]

pub struct HFSStringBinary(pub Vec<u16>);

impl fmt::Debug for HFSStringBinary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for &c in &self.0 {
            if let Some(ch) = core::char::from_u32(c as u32) {
                write!(f, "{}", ch)?;
            } else {
                write!(f, "\\u{{{:04X}}}", c)?;
            }
        }

        Ok(())
    }
}

impl fmt::Display for HFSStringBinary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for &c in &self.0 {
            if let Some(ch) = core::char::from_u32(c as u32) {
                write!(f, "{}", ch)?;
            }
        }

        Ok(())
    }
}

impl PartialOrd for HFSStringBinary {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HFSStringBinary {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl HFSStringTrait for HFSStringBinary {
    fn from_vec(v: Vec<u16>) -> Self {
        HFSStringBinary(v)
    }

    fn to_vec(&self) -> Vec<u16> {
        self.0.clone()
    }
}

pub trait Key: fmt::Debug + Ord + PartialOrd + Eq + PartialEq {
    fn import(source: &mut dyn Read) -> Result<Self>
    where
        Self: Sized;
    fn export(&self, source: &mut dyn Write) -> Result<()>;
}

pub trait Record<K> {
    fn import(source: &mut dyn Read, key: K) -> Result<Self>
    where
        Self: Sized;
    fn export(&self, source: &mut dyn Write) -> Result<()>;
    fn get_key(&self) -> &K;
}

pub struct IndexRecord<K> {
    pub key: K,
    pub node_id: u32,
}

impl<K: Key> Record<K> for IndexRecord<K> {
    fn import(source: &mut dyn Read, key: K) -> Result<Self> {
        let node_id = source.read_u32_be()?;
        Ok(IndexRecord { key, node_id })
    }

    fn export(&self, _source: &mut dyn Write) -> Result<()> {
        Err(Error::UnsupportedOperation)
    }

    fn get_key(&self) -> &K {
        &self.key
    }
}

pub struct HeaderNode {
    pub descriptor: BTNodeDescriptor,
    pub header: BTHeaderRec,
    pub user_data: Vec<u8>,
    pub map: Vec<u8>,
}

pub struct MapNode {
    pub _descriptor: BTNodeDescriptor,
}

pub struct IndexNode<K> {
    pub descriptor: BTNodeDescriptor,
    pub records: Vec<IndexRecord<K>>,
}

pub struct LeafNode<R> {
    pub descriptor: BTNodeDescriptor,
    pub records: Vec<Arc<R>>,
}

pub enum Node<K, R> {
    HeaderNode(HeaderNode),
    MapNode(MapNode),
    IndexNode(IndexNode<K>),
    LeafNode(LeafNode<R>),
}

impl<K: Key, R: Record<K>> Node<K, R> {
    fn load(data: &[u8]) -> Result<Node<K, R>> {
        let mut cursor = Cursor::new(data);
        let node = BTNodeDescriptor::import(&mut cursor)?;
        let num_offsets = (node.num_records + 1) as usize;
        let last_offset_pos = data.len() - num_offsets * 2;
        let mut offsets = Vec::with_capacity(num_offsets);

        for idx in 0..num_offsets {
            let offset_pos = data.len() - 2 - 2 * idx;
            let offset = u16::from_be_bytes([data[offset_pos], data[offset_pos + 1]]) as usize;
            if offset < 14 || offset > last_offset_pos {
                return Err(Error::InvalidData(String::from(
                    "Invalid record offset value",
                )));
            }
            offsets.push(offset);
        }

        let mut records = Vec::new();
        for idx in 0..num_offsets - 1 {
            let first = offsets[idx];
            let last = offsets[idx + 1];
            records.push(&data[first..last]);
        }

        match node.kind {
            BTNodeKind::HeaderNode => {
                let mut r0_cursor = Cursor::new(records[0]);
                Ok(Node::HeaderNode(HeaderNode {
                    descriptor: node,
                    header: BTHeaderRec::import(&mut r0_cursor)?,
                    user_data: records[1].to_vec(),
                    map: records[2].to_vec(),
                }))
            }
            BTNodeKind::MapNode => Ok(Node::MapNode(MapNode { _descriptor: node })),
            BTNodeKind::IndexNode => {
                let mut r = Vec::<IndexRecord<K>>::new();
                for record in &records {
                    let mut v = Cursor::new(record);
                    let r2 = K::import(&mut v)?;
                    r.push(IndexRecord {
                        key: r2,
                        node_id: v.read_u32_be()?,
                    });
                }
                Ok(Node::IndexNode(IndexNode {
                    descriptor: node,
                    records: r,
                }))
            }
            BTNodeKind::LeafNode => {
                let mut r = Vec::<Arc<R>>::new();
                for record in &records {
                    let mut v = Cursor::new(record);
                    let r2 = K::import(&mut v)?;
                    r.push(Arc::new(R::import(&mut v, r2)?));
                }
                Ok(Node::LeafNode(LeafNode {
                    descriptor: node,
                    records: r,
                }))
            }
        }
    }
}

pub struct BTree<F: Read + Seek, K, R> {
    pub fork: F,
    pub node_size: u16,
    pub header: HeaderNode,
    _key: PhantomData<K>,
    _record: PhantomData<R>,
}

impl<F: Read + Seek, K: Key, R: Record<K>> BTree<F, K, R> {
    pub fn open(mut fork: F) -> Result<BTree<F, K, R>> {
        let mut buffer = vec![0; 512];
        fork.seek(SeekFrom::Start(0))?;
        fork.read_exact(&mut buffer)?;
        let node_size = u16::from_be_bytes([buffer[32], buffer[33]]);

        let mut full_buffer = vec![0; node_size as usize];
        full_buffer[..512].copy_from_slice(&buffer);
        fork.seek(SeekFrom::Start(512))?;
        fork.read_exact(&mut full_buffer[512..])?;

        let header_node = Node::<K, R>::load(&full_buffer)?;
        let header = match header_node {
            Node::HeaderNode(x) => x,
            _ => return Err(Error::BadNode),
        };
        Ok(BTree {
            fork,
            node_size,
            header,
            _key: PhantomData,
            _record: PhantomData,
        })
    }

    pub fn get_node(&mut self, node_num: usize) -> Result<Node<K, R>> {
        let mut buffer = vec![0; self.node_size as usize];
        self.fork
            .seek(SeekFrom::Start((node_num * self.node_size as usize) as u64))?;
        self.fork.read_exact(&mut buffer)?;
        Node::<K, R>::load(&buffer)
    }

    pub fn get_record(&mut self, key: &K) -> Result<Arc<R>> {
        self.get_record_node(key, self.header.header.root_node as usize)
    }

    fn get_record_node(&mut self, key: &K, node_id: usize) -> Result<Arc<R>> {
        let node = self.get_node(node_id)?;
        match node {
            Node::IndexNode(x) => {
                let mut return_record = &x.records[0];
                if key < &return_record.key {
                    return Err(Error::InvalidRecordKey);
                }
                for record in x.records.iter().skip(1) {
                    if key < &record.key {
                        break;
                    }
                    return_record = record;
                }

                self.get_record_node(key, return_record.node_id as usize)
            }
            Node::LeafNode(mut x) => loop {
                for record in &x.records {
                    if key < record.get_key() {
                        return Err(Error::KeyNotFound);
                    } else if key == record.get_key() {
                        return Ok(Arc::clone(record));
                    }
                }
                if x.descriptor.f_link == 0 {
                    return Err(Error::KeyNotFound);
                }
                let next_node = self.get_node(x.descriptor.f_link as usize)?;
                x = match next_node {
                    Node::LeafNode(x) => x,
                    _ => return Err(Error::BadNode),
                };
            },
            _ => Err(Error::InvalidRecordType),
        }
    }

    pub fn get_record_range(&mut self, first: &K, last: &K) -> Result<Vec<Arc<R>>> {
        self.get_record_range_node(first, last, self.header.header.root_node as usize)
    }

    fn get_record_range_node(
        &mut self,
        first: &K,
        last: &K,
        node_id: usize,
    ) -> Result<Vec<Arc<R>>> {
        let node = self.get_node(node_id)?;
        match node {
            Node::IndexNode(x) => {
                let mut return_record = &x.records[0];
                if &return_record.key >= last {
                    return Ok(Vec::new());
                }
                for record in x.records.iter().skip(1) {
                    if first < &record.key {
                        break;
                    }
                    return_record = record;
                }
                self.get_record_range_node(first, last, return_record.node_id as usize)
            }
            Node::LeafNode(mut x) => {
                let mut return_records = Vec::new();
                loop {
                    for record in &x.records {
                        if record.get_key() >= last {
                            break;
                        } else if record.get_key() >= first {
                            return_records.push(Arc::clone(record));
                        }
                    }
                    if x.records.is_empty()
                        || x.records[x.records.len() - 1].get_key() >= last
                        || x.descriptor.f_link == 0
                    {
                        break;
                    }
                    let next_node = self.get_node(x.descriptor.f_link as usize)?;
                    x = match next_node {
                        Node::LeafNode(x) => x,
                        _ => return Err(Error::InvalidRecordType),
                    };
                }
                Ok(return_records)
            }
            _ => Err(Error::InvalidRecordType),
        }
    }
}

pub type BTreeArc<F, K, R> = Arc<Mutex<BTree<F, K, R>>>;

pub struct Fork<F: Read + Seek> {
    pub file: Arc<Mutex<F>>,

    pub position: u64,

    pub catalog_id: HFSCatalogNodeID,

    pub fork_type: u8,

    pub block_size: u64,

    pub logical_size: u64,

    pub extents: Vec<(u32, u32, u64, u64)>,

    pub decompressed_cache: Option<Arc<Vec<u8>>>,

    _phantom: PhantomData<F>,
}

impl<F: Read + Seek> Clone for Fork<F> {
    fn clone(&self) -> Self {
        Fork {
            file: Arc::clone(&self.file),

            position: self.position,

            catalog_id: self.catalog_id,

            fork_type: self.fork_type,

            block_size: self.block_size,

            logical_size: self.logical_size,

            extents: self.extents.clone(),

            decompressed_cache: self.decompressed_cache.clone(),

            _phantom: PhantomData,
        }
    }
}

unsafe extern "C" fn zalloc_hfs(
    _opaque: *mut core::ffi::c_void,
    items: core::ffi::c_uint,
    size: core::ffi::c_uint,
) -> *mut core::ffi::c_void {
    let size = items as usize * size as usize;
    let layout = match core::alloc::Layout::from_size_align(size + 16, 16) {
        Ok(l) => l,
        Err(_) => return core::ptr::null_mut(),
    };
    let ptr = unsafe { alloc::alloc::alloc(layout) };
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    unsafe {
        ptr.cast::<usize>().write(size);
        ptr.add(16).cast()
    }
}

unsafe extern "C" fn zfree_hfs(_opaque: *mut core::ffi::c_void, ptr: *mut core::ffi::c_void) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let real_ptr = ptr.sub(16);
        let size = real_ptr.cast::<usize>().read();
        let layout = core::alloc::Layout::from_size_align(size + 16, 16).unwrap();
        alloc::alloc::dealloc(real_ptr.cast(), layout);
    }
}

impl<F: Read + Seek> Fork<F> {
    pub fn load(
        file: Arc<Mutex<F>>,

        catalog_id: HFSCatalogNodeID,

        fork_type: u8,

        volume: &HFSVolume<F>,

        data: &HFSPlusForkData,
    ) -> Result<Fork<F>> {
        let block_size = volume.header.block_size as u64;

        let mut extents = Vec::with_capacity(8);

        let mut extent_position = 0;

        let mut extent_block = 0;

        let mut extents_result = Some(data.extents);

        while let Some(extent_list) = extents_result {
            for extent in &extent_list {
                if extent.block_count == 0 {
                    continue;
                }

                let extent_size = extent.block_count as u64 * block_size;

                let extent_end = extent_position + extent_size;

                let extent_position_clamped = core::cmp::min(data.logical_size, extent_position);

                let extent_end_clamped = core::cmp::min(data.logical_size, extent_end);

                extents.push((
                    extent.start_block,
                    extent.block_count,
                    extent_position_clamped,
                    extent_end_clamped,
                ));

                extent_position += extent_size;

                extent_block += extent.block_count;
            }

            extents_result = None;

            if extent_position < data.logical_size {
                if let Some(et) = &volume.extents_btree {
                    let search_key = ExtentKey::new(catalog_id, fork_type, extent_block);

                    let extent_record = et.lock().get_record(&search_key)?;

                    extents_result = Some(extent_record.body);
                } else {
                    break;
                }
            }
        }

        Ok(Fork {
            file,

            position: 0,

            catalog_id,

            fork_type,

            block_size,

            logical_size: data.logical_size,

            extents,

            decompressed_cache: None,

            _phantom: PhantomData,
        })
    }

    fn decompress_zlib(&self, compressed: &[u8], uncompressed_size: usize) -> Result<Vec<u8>> {
        let mut strm = z_stream {
            next_in: compressed.as_ptr() as *mut _,
            avail_in: compressed.len() as _,
            zalloc: Some(zalloc_hfs),
            zfree: Some(zfree_hfs),
            opaque: core::ptr::null_mut(),
            ..Default::default()
        };

        let config = InflateConfig {
            window_bits: 15, // zlib header
        };

        if init(&mut strm, config) != ReturnCode::Ok {
            return Err(Error::InvalidData(String::from("inflateInit failed")));
        }

        let mut decompressed = vec![
            0u8;
            if uncompressed_size > 0 && uncompressed_size < 512 * 1024 * 1024 {
                uncompressed_size
            } else {
                65536
            }
        ];
        let strm_infl = unsafe { InflateStream::from_stream_mut(&mut strm).unwrap() };

        let mut total_out = 0usize;
        let mut stream_offset = 0usize;

        loop {
            strm.next_out = unsafe { decompressed.as_mut_ptr().add(total_out) };
            strm.avail_out = (decompressed.len() - total_out) as _;

            let ret = unsafe { inflate(strm_infl, zlib_rs::InflateFlush::Finish) };

            let produced_in_session = strm.total_out as usize;
            total_out = stream_offset + produced_in_session;

            if ret == ReturnCode::StreamEnd {
                if strm.avail_in > 0 {
                    // Check magic 0x78
                    if unsafe { *strm.next_in } == 0x78 {
                        stream_offset = total_out;
                        reset(strm_infl);
                        // Ensure we have space for next stream
                        if total_out == decompressed.len() {
                            decompressed.resize(total_out + 65536, 0);
                        }
                        continue;
                    }
                }
                break;
            }

            if ret == ReturnCode::Ok || ret == ReturnCode::BufError {
                if strm.avail_out == 0 {
                    let old_len = decompressed.len();
                    if old_len >= 512 * 1024 * 1024 {
                        end(strm_infl);
                        return Err(Error::InvalidData(String::from(
                            "Decompressed data too large",
                        )));
                    }
                    let grow_by = core::cmp::max(65536, old_len);
                    decompressed.resize(old_len + grow_by, 0);
                    continue;
                }

                if strm.avail_in > 0 {
                    continue;
                }

                break;
            }

            end(strm_infl);
            return Err(Error::InvalidData(format!("inflate failed {:?}", ret)));
        }

        decompressed.truncate(total_out);
        end(strm_infl);
        Ok(decompressed)
    }

    fn decompress_chunked_zlib(&mut self, uncompressed_size: usize) -> Result<Vec<u8>> {
        // Read number of chunks (Little Endian)
        let mut num_chunks_buf = [0u8; 4];
        self.read_exact_internal(&mut num_chunks_buf)?;
        let num_chunks = u32::from_le_bytes(num_chunks_buf) as usize;

        // Read offsets table (Little Endian)
        let mut offsets = Vec::with_capacity(num_chunks);
        for _ in 0..num_chunks {
            let mut offset_buf = [0u8; 4];
            self.read_exact_internal(&mut offset_buf)?;
            offsets.push(u32::from_le_bytes(offset_buf) as usize);
        }

        // The offsets are relative to the start of the chunk table (which started after num_chunks)
        let table_start_pos = self.position - (num_chunks * 4) as u64;
        let mut decompressed = Vec::with_capacity(if uncompressed_size > 0 {
            uncompressed_size
        } else {
            0
        });

        for i in 0..num_chunks {
            let start_offset = offsets[i];
            let end_offset = if i + 1 < num_chunks {
                offsets[i + 1]
            } else {
                // Last chunk: read until the end of the fork
                // Actually, the resource length might be better, but we don't have it easily here.
                // For now, read until end of fork.
                (self.logical_size - table_start_pos) as usize
            };

            if end_offset < start_offset {
                return Err(Error::InvalidData(String::from("Invalid chunk offset")));
            }

            let compressed_size = end_offset - start_offset;
            if compressed_size > 0 {
                let mut compressed_chunk = vec![0u8; compressed_size];
                self.position = table_start_pos + start_offset as u64;
                self.read_exact_internal(&mut compressed_chunk)?;

                // Each chunk is independently compressed with Zlib
                // Each chunk (except the last) decompresses to exactly 64KB (65536 bytes)
                let chunk_uncompressed_size = if i + 1 < num_chunks {
                    65536
                } else if uncompressed_size > decompressed.len() {
                    uncompressed_size - decompressed.len()
                } else {
                    0 // Let decompress_zlib figure it out
                };

                let mut chunk_decompressed =
                    self.decompress_zlib(&compressed_chunk, chunk_uncompressed_size)?;
                decompressed.append(&mut chunk_decompressed);
            }
        }

        Ok(decompressed)
    }

    fn read_to_end(&mut self) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = self.read_internal(&mut buf) {
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
        }
        Ok(data)
    }

    pub fn read_all(&mut self) -> Result<Vec<u8>> {
        let mut buffer = vec![0; self.logical_size as usize];

        self.seek(SeekFrom::Start(0))?;

        self.read_exact(&mut buffer)?;

        Ok(buffer)
    }

    fn check_compression(&mut self) -> Result<()> {
        if self.decompressed_cache.is_some() {
            return Ok(());
        }

        // Only check for compression at the beginning of the file
        if self.position != 0 {
            return Ok(());
        }

        let mut header = [0u8; 16];
        let bytes_read = self.read_internal(&mut header)?;
        if bytes_read < 16 {
            self.position = 0;
            return Ok(());
        }

        let magic = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        if magic == 0x636d7066 {
            // 'cmpf'
            let compression_type = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
            let uncompressed_size = u64::from_be_bytes([
                header[8], header[9], header[10], header[11], header[12], header[13], header[14],
                header[15],
            ]);

            if compression_type == 1 {
                let mut uncompressed = vec![0u8; uncompressed_size as usize];
                self.read_exact_internal(&mut uncompressed)?;
                self.decompressed_cache = Some(Arc::new(uncompressed));
                self.logical_size = uncompressed_size;
                self.position = 0;
                return Ok(());
            } else if compression_type == 3 || compression_type == 4 {
                let compressed = self.read_to_end()?;
                let decompressed = self.decompress_zlib(&compressed, uncompressed_size as usize)?;
                self.decompressed_cache = Some(Arc::new(decompressed));
                self.logical_size = uncompressed_size;
                self.position = 0;
                return Ok(());
            } else if compression_type == 5 {
                let decompressed = self.decompress_chunked_zlib(uncompressed_size as usize)?;
                self.decompressed_cache = Some(Arc::new(decompressed));
                self.logical_size = uncompressed_size;
                self.position = 0;
                return Ok(());
            }
        } else if header[0] == 0x00 && header[1] == 0x00 && header[2] == 0x01 && header[3] == 0x00 {
            // Resource Fork header (AppleDouble / Resource Manager).
            // data_offset is at 0: 00 00 01 00 = 256.
            let data_offset = 256;

            // In a resource fork, the data section contains resources.
            // The first resource starts with a 4-byte length.
            self.position = data_offset as u64;
            let mut res_length_buf = [0u8; 4];
            if self.read_internal(&mut res_length_buf)? == 4 {
                let _res_length = u32::from_be_bytes(res_length_buf);

                // Now we are at the start of the resource data (cmpf header or raw type)
                let mut resource_header = [0u8; 16];
                if self.read_internal(&mut resource_header)? == 16 {
                    let magic = u32::from_be_bytes([
                        resource_header[0],
                        resource_header[1],
                        resource_header[2],
                        resource_header[3],
                    ]);

                    let (compression_type, uncompressed_size, header_size) = if magic == 0x636d7066
                    {
                        (
                            u32::from_be_bytes([
                                resource_header[4],
                                resource_header[5],
                                resource_header[6],
                                resource_header[7],
                            ]),
                            u64::from_be_bytes([
                                resource_header[8],
                                resource_header[9],
                                resource_header[10],
                                resource_header[11],
                                resource_header[12],
                                resource_header[13],
                                resource_header[14],
                                resource_header[15],
                            ]),
                            16usize,
                        )
                    } else if (resource_header[0] == 0x03
                        || resource_header[0] == 0x04
                        || resource_header[0] == 0x05)
                        && resource_header[1] == 0x00
                        && resource_header[2] == 0x00
                        && resource_header[3] == 0x00
                    {
                        // Raw Type 3/4/5 header
                        let raw_type = resource_header[0] as u32;
                        let raw_header_size = u32::from_le_bytes([
                            resource_header[4],
                            resource_header[5],
                            resource_header[6],
                            resource_header[7],
                        ]) as usize;
                        let raw_uncompressed_size = u32::from_le_bytes([
                            resource_header[8],
                            resource_header[9],
                            resource_header[10],
                            resource_header[11],
                        ]) as u64;
                        (
                            raw_type,
                            raw_uncompressed_size,
                            if (4..=256).contains(&raw_header_size) {
                                raw_header_size
                            } else {
                                4
                            },
                        )
                    } else {
                        (0, 0, 0)
                    };

                    if compression_type == 3 || compression_type == 4 {
                        self.position = (data_offset + 4 + header_size) as u64;
                        let compressed = self.read_to_end()?;
                        if let Ok(decompressed) =
                            self.decompress_zlib(&compressed, uncompressed_size as usize)
                        {
                            self.decompressed_cache = Some(Arc::new(decompressed));
                            self.logical_size =
                                self.decompressed_cache.as_ref().unwrap().len() as u64;
                            self.position = 0;
                            return Ok(());
                        }
                    } else if compression_type == 5 {
                        self.position = (data_offset + 4 + header_size) as u64;
                        if let Ok(decompressed) =
                            self.decompress_chunked_zlib(uncompressed_size as usize)
                        {
                            self.decompressed_cache = Some(Arc::new(decompressed));
                            self.logical_size =
                                self.decompressed_cache.as_ref().unwrap().len() as u64;
                            self.position = 0;
                            return Ok(());
                        }
                    } else {
                        // Fallback: Check if the resource data ITSELF is a Zlib stream (some files lack headers)
                        self.position = (data_offset + 4) as u64;
                        let compressed = self.read_to_end()?;
                        if !compressed.is_empty()
                            && compressed[0] == 0x78
                            && let Ok(decompressed) = self.decompress_zlib(&compressed, 0)
                        {
                            self.decompressed_cache = Some(Arc::new(decompressed));
                            self.logical_size =
                                self.decompressed_cache.as_ref().unwrap().len() as u64;
                            self.position = 0;
                            return Ok(());
                        }
                    }
                }
            }
        }

        self.position = 0;
        Ok(())
    }

    fn read_internal(&mut self, buffer: &mut [u8]) -> Result<usize> {
        let offset = self.position;
        let mut file = self.file.lock();
        let block_size = self.block_size;
        let mut bytes_read = 0;

        for extent in &self.extents {
            let (start_block, _, extent_begin, extent_end) = *extent;
            if offset + bytes_read as u64 >= extent_end {
                continue;
            }

            let current_offset = offset + bytes_read as u64;
            let extent_offset = current_offset.saturating_sub(extent_begin);

            file.seek(SeekFrom::Start(
                start_block as u64 * block_size + extent_offset,
            ))?;

            let bytes_remaining = buffer.len() - bytes_read;
            let available_in_extent = extent_end - current_offset;
            let bytes_to_read = core::cmp::min(available_in_extent, bytes_remaining as u64);

            file.read_exact(&mut buffer[bytes_read..bytes_read + bytes_to_read as usize])?;
            bytes_read += bytes_to_read as usize;

            if bytes_read >= buffer.len() {
                break;
            }
        }

        self.position += bytes_read as u64;
        Ok(bytes_read)
    }

    fn read_exact_internal(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.read_internal(buf) {
                Ok(0) => break,
                Ok(n) => {
                    let tmp = buf;
                    buf = &mut tmp[n..];
                }
                Err(e) => return Err(e),
            }
        }
        if !buf.is_empty() {
            Err(Error::InvalidData(String::from("Unexpected EOF")))
        } else {
            Ok(())
        }
    }
}

impl<F: Read + Seek> Read for Fork<F> {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.check_compression()?;

        if let Some(cache) = &self.decompressed_cache {
            let offset = self.position as usize;
            if offset >= cache.len() {
                return Ok(0);
            }
            let to_copy = core::cmp::min(buffer.len(), cache.len() - offset);
            buffer[..to_copy].copy_from_slice(&cache[offset..offset + to_copy]);
            self.position += to_copy as u64;
            return Ok(to_copy);
        }

        self.read_internal(buffer)
    }
}

impl<F: Read + Seek> Seek for Fork<F> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let new_position = match pos {
            SeekFrom::Start(x) => x,

            SeekFrom::Current(x) => (self.position as i64 + x) as u64,

            SeekFrom::End(x) => (self.logical_size as i64 + x) as u64,
        };

        self.position = new_position;

        Ok(new_position)
    }
}

pub enum CatalogBTreeEnum<F: Read + Seek> {
    CaseFolding(BTreeArc<Fork<F>, CatalogKey<HFSString>, CatalogRecord<HFSString>>),

    Binary(BTreeArc<Fork<F>, CatalogKey<HFSStringBinary>, CatalogRecord<HFSStringBinary>>),
}

fn convert_key(k: CatalogKey<HFSStringBinary>) -> CatalogKey<HFSString> {
    CatalogKey {
        _case_match: k._case_match,
        parent_id: k.parent_id,
        node_name: HFSString(k.node_name.to_vec()),
    }
}

fn convert_record(rec: CatalogRecord<HFSStringBinary>) -> CatalogRecord<HFSString> {
    CatalogRecord {
        key: convert_key(rec.key),
        body: match rec.body {
            CatalogBody::Folder(f) => CatalogBody::Folder(f),
            CatalogBody::File(f) => CatalogBody::File(f),
            CatalogBody::FolderThread(k) => CatalogBody::FolderThread(convert_key(k)),
            CatalogBody::FileThread(k) => CatalogBody::FileThread(convert_key(k)),
        },
    }
}

pub struct HFSVolume<F: Read + Seek> {
    pub file: Arc<Mutex<F>>,

    pub header: HFSPlusVolumeHeader,

    pub catalog_btree: Option<CatalogBTreeEnum<F>>,

    pub extents_btree: Option<BTreeArc<Fork<F>, ExtentKey, ExtentRecord>>,
}

impl<F: Read + Seek> HFSVolume<F> {
    pub fn load(mut file: F) -> Result<Arc<Mutex<HFSVolume<F>>>> {
        file.seek(SeekFrom::Start(1024))?;

        let header = HFSPlusVolumeHeader::import(&mut file)?;

        if header.signature != HFSP_SIGNATURE && header.signature != HFSX_SIGNATURE {
            return Err(Error::InvalidData(String::from("Invalid volume signature")));
        }

        let file_arc = Arc::new(Mutex::new(file));

        let volume = Arc::new(Mutex::new(HFSVolume {
            file: file_arc,

            header,

            catalog_btree: None,

            extents_btree: None,
        }));

        let catalog_data = volume.lock().header.catalog_file;

        let file_clone = Arc::clone(&volume.lock().file);

        let catalog_fork = {
            let vol_guard = volume.lock();

            Fork::load(
                file_clone,
                K_HFSCATALOG_FILE_ID,
                0,
                &*vol_guard,
                &catalog_data,
            )?
        };

        let temp_btree = BTree::<Fork<F>, CatalogKey<HFSString>, CatalogRecord<HFSString>>::open(
            catalog_fork.clone(),
        )?;

        let compare_type = temp_btree.header.header.key_compare_type;

        let catalog_enum = if compare_type == 0xBC {
            let btree = BTree::<
                Fork<F>,
                CatalogKey<HFSStringBinary>,
                CatalogRecord<HFSStringBinary>,
            >::open(catalog_fork)?;

            CatalogBTreeEnum::Binary(Arc::new(Mutex::new(btree)))
        } else {
            CatalogBTreeEnum::CaseFolding(Arc::new(Mutex::new(temp_btree)))
        };

        volume.lock().catalog_btree = Some(catalog_enum);

        let extents_data = volume.lock().header.extents_file;

        let file_clone_ext = Arc::clone(&volume.lock().file);

        let extents_fork = {
            let vol_guard = volume.lock();

            Fork::load(
                file_clone_ext,
                K_HFSEXTENTS_FILE_ID,
                0,
                &*vol_guard,
                &extents_data,
            )?
        };

        volume.lock().extents_btree = Some(Arc::new(Mutex::new(BTree::open(extents_fork)?)));

        Ok(volume)
    }

    pub fn get_path_record(&self, filename: &str) -> Result<CatalogRecord> {
        match self.catalog_btree.as_ref().unwrap() {
            CatalogBTreeEnum::CaseFolding(btree) => {
                self.get_path_record_impl(filename, &mut *btree.lock())
            }

            CatalogBTreeEnum::Binary(btree) => {
                let rec = self.get_path_record_impl(filename, &mut *btree.lock())?;

                Ok(convert_record(rec))
            }
        }
    }

    fn get_path_record_impl<S>(
        &self,
        filename: &str,
        btree: &mut BTree<Fork<F>, CatalogKey<S>, CatalogRecord<S>>,
    ) -> Result<CatalogRecord<S>>
    where
        S: HFSStringTrait,
    {
        let parts: Vec<&str> = filename.split('/').filter(|s| !s.is_empty()).collect();

        let mut current_folder_id = 2; // kHFSRootFolderID

        let mut current_record: Option<CatalogRecord<S>> = None;

        if parts.is_empty() {
            // Return root record (Thread (2, "") -> Real (1, "VolumeName"))

            let thread_key = CatalogKey {
                _case_match: false,

                parent_id: 2,

                node_name: S::from_vec(vec![]),
            };

            match btree.get_record(&thread_key) {
                Ok(record) => {
                    if let CatalogBody::FolderThread(ref thread_data) = record.body {
                        let real_record = btree.get_record(thread_data)?;

                        return Ok((*real_record).clone());
                    }
                }

                Err(_) => {

                    // Fallback: search parent 1
                }
            }
        }

        for (i, part) in parts.iter().enumerate() {
            let name_utf16: Vec<u16> = part.nfd().collect::<String>().encode_utf16().collect();

            let key = CatalogKey {
                _case_match: false,

                parent_id: current_folder_id,

                node_name: S::from_vec(name_utf16),
            };

            let record = btree.get_record(&key)?;

            current_record = Some((*record).clone());

            match &record.body {
                CatalogBody::Folder(f) => {
                    current_folder_id = f.folder_id;
                }

                CatalogBody::File(_) => {
                    if i != parts.len() - 1 {
                        return Err(Error::KeyNotFound);
                    }
                }

                _ => return Err(Error::InvalidRecordType),
            }
        }

        current_record.ok_or(Error::KeyNotFound)
    }

    pub fn list_dir(&self, path: &str) -> Result<Vec<(String, CatalogRecord)>> {
        let record = self.get_path_record(path)?;

        let folder_id = match record.body {
            CatalogBody::Folder(f) => f.folder_id,

            _ => return Err(Error::InvalidRecordType),
        };

        match self.catalog_btree.as_ref().unwrap() {
            CatalogBTreeEnum::CaseFolding(btree) => {
                self.list_dir_impl(folder_id, &mut *btree.lock())
            }

            CatalogBTreeEnum::Binary(btree) => {
                let results = self.list_dir_impl(folder_id, &mut *btree.lock())?;

                Ok(results
                    .into_iter()
                    .map(|(n, r)| (n, convert_record(r)))
                    .collect())
            }
        }
    }

    fn list_dir_impl<S>(
        &self,
        folder_id: HFSCatalogNodeID,
        btree: &mut BTree<Fork<F>, CatalogKey<S>, CatalogRecord<S>>,
    ) -> Result<Vec<(String, CatalogRecord<S>)>>
    where
        S: HFSStringTrait,
    {
        let first_key = CatalogKey {
            _case_match: false,

            parent_id: folder_id,

            node_name: S::from_vec(vec![]),
        };

        let last_key = CatalogKey {
            _case_match: false,

            parent_id: folder_id + 1,

            node_name: S::from_vec(vec![]),
        };

        let records = btree.get_record_range(&first_key, &last_key)?;

        let mut results = Vec::new();

        for r in records {
            if r.key.parent_id == folder_id {
                results.push((format!("{}", r.key.node_name), (*r).clone()));
            }
        }

        Ok(results)
    }
}
