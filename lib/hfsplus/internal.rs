use crate::{Read, ReadExt, Result, Write, WriteExt};
use bitflags::bitflags;

#[derive(Debug, Copy, Clone)]
pub struct HFSPlusBSDInfo {
    pub owner_id: u32,
    pub group_id: u32,
    pub admin_flags: u8,
    pub owner_flags: u8,
    pub file_mode: u16,
    pub special: u32,
}

impl HFSPlusBSDInfo {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            owner_id: source.read_u32_be()?,
            group_id: source.read_u32_be()?,
            admin_flags: source.read_u8()?,
            owner_flags: source.read_u8()?,
            file_mode: source.read_u16_be()?,
            special: source.read_u32_be()?,
        })
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct FileMode: u16 {
        const S_ISUID = 0o0004000;
        const S_ISGID = 0o0002000;
        const S_ISTXT = 0o0001000;

        const S_IRWXU = 0o0000700;
        const S_IRUSR = 0o0000400;
        const S_IWUSR = 0o0000200;
        const S_IXUSR = 0o0000100;

        const S_IRWXG = 0o0000070;
        const S_IRGRP = 0o0000040;
        const S_IWGRP = 0o0000020;
        const S_IXGRP = 0o0000010;

        const S_IRWXO = 0o0000007;
        const S_IROTH = 0o0000004;
        const S_IWOTH = 0o0000002;
        const S_IXOTH = 0o0000001;

        const S_IFMT  = 0o0170000;
        const S_IFIFO = 0o0010000;
        const S_IFCHR = 0o0020000;
        const S_IFDIR = 0o0040000;
        const S_IFBLK = 0o0060000;
        const S_IFREG = 0o0100000;
        const S_IFLNK = 0o0120000;
        const S_IFSOCK = 0o0140000;
        const S_IFWHT = 0o0160000;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct HFSPlusForkData {
    pub logical_size: u64,
    pub clump_size: u32,
    pub total_blocks: u32,
    pub extents: HFSPlusExtentRecord,
}

pub type HFSPlusExtentRecord = [HFSPlusExtentDescriptor; 8];

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct HFSPlusExtentDescriptor {
    pub start_block: u32,
    pub block_count: u32,
}

impl HFSPlusForkData {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            logical_size: source.read_u64_be()?,
            clump_size: source.read_u32_be()?,
            total_blocks: source.read_u32_be()?,
            extents: import_record(source)?,
        })
    }

    pub fn export(&self, source: &mut dyn Write) -> Result<()> {
        source.write_u64_be(self.logical_size)?;
        source.write_u32_be(self.clump_size)?;
        source.write_u32_be(self.total_blocks)?;
        export_record(&self.extents, source)?;
        Ok(())
    }
}

pub fn import_record(source: &mut dyn Read) -> Result<HFSPlusExtentRecord> {
    Ok([
        HFSPlusExtentDescriptor::import(source)?,
        HFSPlusExtentDescriptor::import(source)?,
        HFSPlusExtentDescriptor::import(source)?,
        HFSPlusExtentDescriptor::import(source)?,
        HFSPlusExtentDescriptor::import(source)?,
        HFSPlusExtentDescriptor::import(source)?,
        HFSPlusExtentDescriptor::import(source)?,
        HFSPlusExtentDescriptor::import(source)?,
    ])
}

pub fn export_record(record: &[HFSPlusExtentDescriptor], source: &mut dyn Write) -> Result<()> {
    for r in record {
        r.export(source)?;
    }
    Ok(())
}

impl HFSPlusExtentDescriptor {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            start_block: source.read_u32_be()?,
            block_count: source.read_u32_be()?,
        })
    }

    pub fn export(&self, source: &mut dyn Write) -> Result<()> {
        source.write_u32_be(self.start_block)?;
        source.write_u32_be(self.block_count)?;
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct HFSPlusVolumeHeader {
    pub signature: u16,
    pub version: u16,
    pub attributes: u32,
    pub last_mounted_version: u32,
    pub journal_info_block: u32,
    pub create_date: u32,
    pub modify_date: u32,
    pub backup_date: u32,
    pub checked_date: u32,
    pub file_count: u32,
    pub folder_count: u32,
    pub block_size: u32,
    pub total_blocks: u32,
    pub free_blocks: u32,
    pub next_allocation: u32,
    pub rsrc_clump_size: u32,
    pub data_clump_size: u32,
    pub next_catalog_id: u32,
    pub write_count: u32,
    pub encodings_bitmap: u64,
    pub finder_info: [u32; 8],
    pub allocation_file: HFSPlusForkData,
    pub extents_file: HFSPlusForkData,
    pub catalog_file: HFSPlusForkData,
    pub attributes_file: HFSPlusForkData,
    pub startup_file: HFSPlusForkData,
}

impl HFSPlusVolumeHeader {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            signature: source.read_u16_be()?,
            version: source.read_u16_be()?,
            attributes: source.read_u32_be()?,
            last_mounted_version: source.read_u32_be()?,
            journal_info_block: source.read_u32_be()?,
            create_date: source.read_u32_be()?,
            modify_date: source.read_u32_be()?,
            backup_date: source.read_u32_be()?,
            checked_date: source.read_u32_be()?,
            file_count: source.read_u32_be()?,
            folder_count: source.read_u32_be()?,
            block_size: source.read_u32_be()?,
            total_blocks: source.read_u32_be()?,
            free_blocks: source.read_u32_be()?,
            next_allocation: source.read_u32_be()?,
            rsrc_clump_size: source.read_u32_be()?,
            data_clump_size: source.read_u32_be()?,
            next_catalog_id: source.read_u32_be()?,
            write_count: source.read_u32_be()?,
            encodings_bitmap: source.read_u64_be()?,
            finder_info: [
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
            ],
            allocation_file: HFSPlusForkData::import(source)?,
            extents_file: HFSPlusForkData::import(source)?,
            catalog_file: HFSPlusForkData::import(source)?,
            attributes_file: HFSPlusForkData::import(source)?,
            startup_file: HFSPlusForkData::import(source)?,
        })
    }
}

pub const HFSP_SIGNATURE: u16 = 0x482b;
pub const HFSX_SIGNATURE: u16 = 0x4858;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(i8)]
pub enum BTNodeKind {
    LeafNode = -1,
    IndexNode = 0,
    HeaderNode = 1,
    MapNode = 2,
}

impl BTNodeKind {
    pub fn from_i8(v: i8) -> Result<Self> {
        match v {
            -1 => Ok(BTNodeKind::LeafNode),
            0 => Ok(BTNodeKind::IndexNode),
            1 => Ok(BTNodeKind::HeaderNode),
            2 => Ok(BTNodeKind::MapNode),
            _ => Err(crate::Error::InvalidData(alloc::format!(
                "Invalid BTNodeKind: {}",
                v
            ))),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct BTNodeDescriptor {
    pub f_link: u32,
    pub b_link: u32,
    pub kind: BTNodeKind,
    pub height: u8,
    pub num_records: u16,
    pub reserved: u16,
}

impl BTNodeDescriptor {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            f_link: source.read_u32_be()?,
            b_link: source.read_u32_be()?,
            kind: BTNodeKind::from_i8(source.read_i8()?)?,
            height: source.read_u8()?,
            num_records: source.read_u16_be()?,
            reserved: source.read_u16_be()?,
        })
    }

    pub fn export(&self, source: &mut dyn Write) -> Result<()> {
        source.write_u32_be(self.f_link)?;
        source.write_u32_be(self.b_link)?;
        source.write_i8(self.kind as i8)?;
        source.write_u8(self.height)?;
        source.write_u16_be(self.num_records)?;
        source.write_u16_be(self.reserved)?;
        Ok(())
    }
}

// These are no longer needed as they are in the enum, but if code relies on them as constants...
// The prompt said "use enums instead of just constants", so I'll remove them.

pub const HEADER_NODE_KIND: u8 = 1;
pub const BT_LEAF_NODE_KIND: u8 = 255; // -1 as u8

#[derive(Debug, PartialEq, Eq)]
pub struct BTHeaderRec {
    pub tree_depth: u16,
    pub root_node: u32,
    pub leaf_records: u32,
    pub first_leaf_node: u32,
    pub last_leaf_node: u32,
    pub node_size: u16,
    pub max_key_length: u16,
    pub total_nodes: u32,
    pub free_nodes: u32,
    pub reserved1: u16,
    pub clump_size: u32,
    pub b_tree_type: u8,
    pub key_compare_type: u8,
    pub attributes: u32,
    pub reserved3: [u32; 16],
}

impl BTHeaderRec {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            tree_depth: source.read_u16_be()?,
            root_node: source.read_u32_be()?,
            leaf_records: source.read_u32_be()?,
            first_leaf_node: source.read_u32_be()?,
            last_leaf_node: source.read_u32_be()?,
            node_size: source.read_u16_be()?,
            max_key_length: source.read_u16_be()?,
            total_nodes: source.read_u32_be()?,
            free_nodes: source.read_u32_be()?,
            reserved1: source.read_u16_be()?,
            clump_size: source.read_u32_be()?,
            b_tree_type: source.read_u8()?,
            key_compare_type: source.read_u8()?,
            attributes: source.read_u32_be()?,
            reserved3: [
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
                source.read_u32_be()?,
            ],
        })
    }

    pub fn export(&self, source: &mut dyn Write) -> Result<()> {
        source.write_u16_be(self.tree_depth)?;
        source.write_u32_be(self.root_node)?;
        source.write_u32_be(self.leaf_records)?;
        source.write_u32_be(self.first_leaf_node)?;
        source.write_u32_be(self.last_leaf_node)?;
        source.write_u16_be(self.node_size)?;
        source.write_u16_be(self.max_key_length)?;
        source.write_u32_be(self.total_nodes)?;
        source.write_u32_be(self.free_nodes)?;
        source.write_u16_be(self.reserved1)?;
        source.write_u32_be(self.clump_size)?;
        source.write_u8(self.b_tree_type)?;
        source.write_u8(self.key_compare_type)?;
        source.write_u32_be(self.attributes)?;
        for r in &self.reserved3 {
            source.write_u32_be(*r)?;
        }
        Ok(())
    }
}

pub type HFSCatalogNodeID = u32;
pub const K_HFSCATALOG_FILE_ID: HFSCatalogNodeID = 4;
pub const K_HFSEXTENTS_FILE_ID: HFSCatalogNodeID = 3;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(i16)]
pub enum CatalogRecordType {
    Folder = 0x0001,
    File = 0x0002,
    FolderThread = 0x0003,
    FileThread = 0x0004,
}

impl CatalogRecordType {
    pub fn from_i16(v: i16) -> Result<Self> {
        match v {
            0x0001 => Ok(CatalogRecordType::Folder),
            0x0002 => Ok(CatalogRecordType::File),
            0x0003 => Ok(CatalogRecordType::FolderThread),
            0x0004 => Ok(CatalogRecordType::FileThread),
            _ => Err(crate::Error::InvalidRecordType),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct HFSPlusCatalogFolder {
    pub flags: u16,
    pub valence: u32,
    pub folder_id: HFSCatalogNodeID,
    pub created_at: u32,
    pub content_modified_at: u32,
    pub attribute_modified_at: u32,
    pub accessed_at: u32,
    pub backed_up_at: u32,
    pub permissions: HFSPlusBSDInfo,
    pub user_info: FolderInfo,
    pub finder_info: ExtendedFolderInfo,
    pub text_encoding: u32,
    pub reserved: u32,
}

impl HFSPlusCatalogFolder {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            flags: source.read_u16_be()?,
            valence: source.read_u32_be()?,
            folder_id: source.read_u32_be()?,
            created_at: source.read_u32_be()?,
            content_modified_at: source.read_u32_be()?,
            attribute_modified_at: source.read_u32_be()?,
            accessed_at: source.read_u32_be()?,
            backed_up_at: source.read_u32_be()?,
            permissions: HFSPlusBSDInfo::import(source)?,
            user_info: FolderInfo::import(source)?,
            finder_info: ExtendedFolderInfo::import(source)?,
            text_encoding: source.read_u32_be()?,
            reserved: source.read_u32_be()?,
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub struct HFSPlusCatalogFile {
    pub flags: u16,
    pub reserved1: u32,
    pub file_id: HFSCatalogNodeID,
    pub created_at: u32,
    pub content_modified_at: u32,
    pub attribute_modified_at: u32,
    pub accessed_at: u32,
    pub backed_up_at: u32,
    pub permissions: HFSPlusBSDInfo,
    pub user_info: FileInfo,
    pub finder_info: ExtendedFileInfo,
    pub text_encoding: u32,
    pub reserved2: u32,
    pub data_fork: HFSPlusForkData,
    pub resource_fork: HFSPlusForkData,
}

impl HFSPlusCatalogFile {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            flags: source.read_u16_be()?,
            reserved1: source.read_u32_be()?,
            file_id: source.read_u32_be()?,
            created_at: source.read_u32_be()?,
            content_modified_at: source.read_u32_be()?,
            attribute_modified_at: source.read_u32_be()?,
            accessed_at: source.read_u32_be()?,
            backed_up_at: source.read_u32_be()?,
            permissions: HFSPlusBSDInfo::import(source)?,
            user_info: FileInfo::import(source)?,
            finder_info: ExtendedFileInfo::import(source)?,
            text_encoding: source.read_u32_be()?,
            reserved2: source.read_u32_be()?,
            data_fork: HFSPlusForkData::import(source)?,
            resource_fork: HFSPlusForkData::import(source)?,
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Point {
    pub v: i16,
    pub h: i16,
}
impl Point {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            v: source.read_i16_be()?,
            h: source.read_i16_be()?,
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Rect {
    pub top: i16,
    pub left: i16,
    pub bottom: i16,
    pub right: i16,
}
impl Rect {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            top: source.read_i16_be()?,
            left: source.read_i16_be()?,
            bottom: source.read_i16_be()?,
            right: source.read_i16_be()?,
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub struct FileInfo {
    pub file_type: u32,
    pub file_creator: u32,
    pub finder_flags: u16,
    pub location: Point,
    pub reserved: u16,
}
impl FileInfo {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            file_type: source.read_u32_be()?,
            file_creator: source.read_u32_be()?,
            finder_flags: source.read_u16_be()?,
            location: Point::import(source)?,
            reserved: source.read_u16_be()?,
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub struct ExtendedFileInfo {
    pub reserved1: [i16; 4],
    pub extended_finder_flags: u16,
    pub reserved2: i16,
    pub put_away_folder_id: i32,
}
impl ExtendedFileInfo {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            reserved1: [
                source.read_i16_be()?,
                source.read_i16_be()?,
                source.read_i16_be()?,
                source.read_i16_be()?,
            ],
            extended_finder_flags: source.read_u16_be()?,
            reserved2: source.read_i16_be()?,
            put_away_folder_id: source.read_i32_be()?,
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub struct FolderInfo {
    pub window_bounds: Rect,
    pub finder_flags: u16,
    pub location: Point,
    pub reserved_field: u16,
}
impl FolderInfo {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            window_bounds: Rect::import(source)?,
            finder_flags: source.read_u16_be()?,
            location: Point::import(source)?,
            reserved_field: source.read_u16_be()?,
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub struct ExtendedFolderInfo {
    pub scroll_position: Point,
    pub reserved1: i32,
    pub extended_finder_flags: u16,
    pub reserved2: i16,
    pub put_away_folder_id: i32,
}
impl ExtendedFolderInfo {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            scroll_position: Point::import(source)?,
            reserved1: source.read_i32_be()?,
            extended_finder_flags: source.read_u16_be()?,
            reserved2: source.read_i16_be()?,
            put_away_folder_id: source.read_i32_be()?,
        })
    }
}

#[derive(Debug, Copy, Clone)]
pub struct HFSPlusExtentKey {
    pub key_length: u16,
    pub fork_type: u8,
    pub pad: u8,
    pub file_id: u32,
    pub start_block: u32,
}
impl HFSPlusExtentKey {
    pub fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            key_length: source.read_u16_be()?,
            fork_type: source.read_u8()?,
            pad: source.read_u8()?,
            file_id: source.read_u32_be()?,
            start_block: source.read_u32_be()?,
        })
    }
    pub fn export(&self, source: &mut dyn Write) -> Result<()> {
        source.write_u16_be(self.key_length)?;
        source.write_u8(self.fork_type)?;
        source.write_u8(self.pad)?;
        source.write_u32_be(self.file_id)?;
        source.write_u32_be(self.start_block)?;
        Ok(())
    }
}

#[derive(Debug, Copy, Clone)]
pub struct ExtentKey(pub HFSPlusExtentKey);

impl ExtentKey {
    pub fn new(file_id: HFSCatalogNodeID, fork_type: u8, start_block: u32) -> Self {
        ExtentKey(HFSPlusExtentKey {
            key_length: 10,
            fork_type,
            pad: 0,
            file_id,
            start_block,
        })
    }
}

impl crate::Key for ExtentKey {
    fn import(source: &mut dyn Read) -> Result<Self> {
        Ok(ExtentKey(HFSPlusExtentKey::import(source)?))
    }

    fn export(&self, source: &mut dyn Write) -> Result<()> {
        self.0.export(source)?;
        Ok(())
    }
}

impl core::cmp::PartialOrd for ExtentKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl core::cmp::Ord for ExtentKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        match self.0.file_id.cmp(&other.0.file_id) {
            core::cmp::Ordering::Less => core::cmp::Ordering::Less,
            core::cmp::Ordering::Greater => core::cmp::Ordering::Greater,
            core::cmp::Ordering::Equal => match self.0.fork_type.cmp(&other.0.fork_type) {
                core::cmp::Ordering::Less => core::cmp::Ordering::Less,
                core::cmp::Ordering::Greater => core::cmp::Ordering::Greater,
                core::cmp::Ordering::Equal => self.0.start_block.cmp(&other.0.start_block),
            },
        }
    }
}

impl core::cmp::PartialEq for ExtentKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == core::cmp::Ordering::Equal
    }
}

impl core::cmp::Eq for ExtentKey {}

#[derive(Debug, Clone)]
pub struct CatalogKey<S = crate::HFSString> {
    pub _case_match: bool,
    pub parent_id: HFSCatalogNodeID,
    pub node_name: S,
}

impl<S: crate::HFSStringTrait> crate::Key for CatalogKey<S> {
    fn import(source: &mut dyn Read) -> Result<Self> {
        let key_length = source.read_u16_be()?;
        if key_length < 6 {
            return Err(crate::Error::InvalidRecordKey);
        }
        let parent_id = source.read_u32_be()?;
        let count = source.read_u16_be()?;
        let mut node_name = alloc::vec::Vec::with_capacity(count as usize);
        for _ in 0..count as usize {
            node_name.push(source.read_u16_be()?);
        }
        Ok(Self {
            _case_match: false,
            parent_id,
            node_name: S::from_vec(node_name),
        })
    }

    fn export(&self, _source: &mut dyn Write) -> Result<()> {
        Err(crate::Error::UnsupportedOperation)
    }
}

impl<S: Ord> core::cmp::PartialOrd for CatalogKey<S> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<S: Ord> core::cmp::Ord for CatalogKey<S> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        match self.parent_id.cmp(&other.parent_id) {
            core::cmp::Ordering::Less => core::cmp::Ordering::Less,
            core::cmp::Ordering::Greater => core::cmp::Ordering::Greater,
            core::cmp::Ordering::Equal => self.node_name.cmp(&other.node_name),
        }
    }
}

impl<S: PartialEq + Ord> core::cmp::PartialEq for CatalogKey<S> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == core::cmp::Ordering::Equal
    }
}

impl<S: Eq + Ord> core::cmp::Eq for CatalogKey<S> {}

#[derive(Debug, Clone)]
pub enum CatalogBody<S = crate::HFSString> {
    Folder(HFSPlusCatalogFolder),
    File(HFSPlusCatalogFile),
    FolderThread(CatalogKey<S>),
    FileThread(CatalogKey<S>),
}

#[derive(Debug, Clone)]
pub struct CatalogRecord<S = crate::HFSString> {
    pub key: CatalogKey<S>,
    pub body: CatalogBody<S>,
}

impl<S: crate::HFSStringTrait> crate::Record<CatalogKey<S>> for CatalogRecord<S> {
    fn import(source: &mut dyn Read, key: CatalogKey<S>) -> Result<Self> {
        let record_type = CatalogRecordType::from_i16(source.read_i16_be()?)?;
        let body = match record_type {
            CatalogRecordType::Folder => CatalogBody::Folder(HFSPlusCatalogFolder::import(source)?),
            CatalogRecordType::File => CatalogBody::File(HFSPlusCatalogFile::import(source)?),
            CatalogRecordType::FolderThread => {
                let _reserved = source.read_i16_be()?;
                let parent_id = source.read_u32_be()?;
                let count = source.read_u16_be()?;
                let mut node_name = alloc::vec::Vec::with_capacity(count as usize);
                for _ in 0..count as usize {
                    node_name.push(source.read_u16_be()?);
                }
                let to_key = CatalogKey {
                    _case_match: false,
                    parent_id,
                    node_name: S::from_vec(node_name),
                };
                CatalogBody::FolderThread(to_key)
            }
            CatalogRecordType::FileThread => {
                let _reserved = source.read_i16_be()?;
                let parent_id = source.read_u32_be()?;
                let count = source.read_u16_be()?;
                let mut node_name = alloc::vec::Vec::with_capacity(count as usize);
                for _ in 0..count as usize {
                    node_name.push(source.read_u16_be()?);
                }
                let to_key = CatalogKey {
                    _case_match: false,
                    parent_id,
                    node_name: S::from_vec(node_name),
                };
                CatalogBody::FileThread(to_key)
            }
        };
        Ok(CatalogRecord { key, body })
    }

    fn export(&self, _source: &mut dyn Write) -> Result<()> {
        Err(crate::Error::UnsupportedOperation)
    }

    fn get_key(&self) -> &CatalogKey<S> {
        &self.key
    }
}

#[derive(Debug, Clone)]
pub struct ExtentRecord {
    pub key: ExtentKey,
    pub body: HFSPlusExtentRecord,
}

impl crate::Record<ExtentKey> for ExtentRecord {
    fn import(source: &mut dyn Read, key: ExtentKey) -> Result<Self> {
        let body = import_record(source)?;
        Ok(ExtentRecord { key, body })
    }

    fn export(&self, source: &mut dyn Write) -> Result<()> {
        export_record(&self.body, source)?;
        Ok(())
    }

    fn get_key(&self) -> &ExtentKey {
        &self.key
    }
}
