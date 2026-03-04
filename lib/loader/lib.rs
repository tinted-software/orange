#![no_std]

#[cfg(not(target_os = "none"))]
extern crate std;

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use core::mem::size_of;

use goblin::mach::constants::cputype::CPU_TYPE_ARM64;
use goblin::mach::fat;
use goblin::mach::load_command::CommandVariant;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("truncated file")]
    TruncatedFile,
    #[error("unsupported binary format")]
    UnsupportedBinaryFormat,
    #[error("arm64 slice not found in fat binary")]
    Arm64SliceNotFound,
    #[error("not a Mach-O 64-bit binary")]
    NotMachO64,
    #[error("not arm64")]
    NotArm64,
    #[error("invalid load command")]
    InvalidLoadCommand,
    #[error("missing entry point")]
    MissingEntryPoint,
    #[error("invalid chained fixups")]
    InvalidChainedFixups,
    #[error("invalid chained starts")]
    InvalidChainedStarts,
    #[error("invalid chained imports")]
    InvalidChainedImports,
    #[error("invalid chained symbols")]
    InvalidChainedSymbols,
    #[error("invalid chained import ordinal")]
    InvalidChainedImportOrdinal,
    #[error("unsupported chained pointer format")]
    UnsupportedChainedPointerFormat,
    #[error("unsupported chained imports format")]
    UnsupportedChainedImportsFormat,
    #[error("unsupported fixups version")]
    UnsupportedFixupsVersion,
    #[error("unsupported fixups symbols format")]
    UnsupportedFixupsSymbolsFormat,
    #[error("invalid chained pointer location")]
    InvalidChainedPointerLocation,
    #[error("truncated chained fixups")]
    TruncatedChainedFixups,
    #[error("invalid segment index")]
    InvalidSegmentIndex,
    #[error("invalid segment offset")]
    InvalidSegmentOffset,
    #[error("invalid segment layout")]
    InvalidSegmentLayout,
    #[error("no mappable segments")]
    NoMappableSegments,
    #[error("image too large")]
    ImageTooLarge,
    #[error("truncated symtab")]
    TruncatedSymtab,
    #[error("truncated strtab")]
    TruncatedStrtab,
    #[error("invalid undefined symbol range")]
    InvalidUndefinedSymbolRange,
    #[error("invalid symbol string offset")]
    InvalidSymbolStringOffset,
    #[error("invalid export trie")]
    InvalidExportTrie,
    #[error("no export trie")]
    NoExportTrie,
    #[error("truncated dyld info")]
    TruncatedDyldInfo,
    #[error("unsupported rebase opcode")]
    UnsupportedRebaseOpcode,
    #[error("unsupported rebase type")]
    UnsupportedRebaseType,
    #[error("unsupported bind opcode")]
    UnsupportedBindOpcode,
    #[error("unsupported bind type")]
    UnsupportedBindType,
    #[error("unresolved chained import: {0}")]
    UnresolvedChainedImport(String),
    #[error("unresolved bind symbol: {0}")]
    UnresolvedBindSymbol(String),
    #[error("invalid LEB128")]
    InvalidLeb128,
    #[error("entry point outside segments")]
    EntryOutsideSegments,
    #[error("goblin: {0}")]
    Goblin(#[from] goblin::error::Error),
}

pub type Result<T> = core::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Load command data
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct LinkeditDataCmd {
    pub dataoff: u32,
    pub datasize: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct DyldInfoCmd {
    pub rebase_off: u32,
    pub rebase_size: u32,
    pub bind_off: u32,
    pub bind_size: u32,
    pub weak_bind_off: u32,
    pub weak_bind_size: u32,
    pub lazy_bind_off: u32,
    pub lazy_bind_size: u32,
    pub export_off: u32,
    pub export_size: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct SymtabCmd {
    pub symoff: u32,
    pub nsyms: u32,
    pub stroff: u32,
    pub strsize: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct DysymtabCmd {
    pub iundefsym: u32,
    pub nundefsym: u32,
}

// ---------------------------------------------------------------------------
// Segment & load plan
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SegmentPlan {
    pub name: [u8; 16],
    pub vmaddr: u64,
    pub vmsize: u64,
    pub fileoff: u64,
    pub filesize: u64,
    pub maxprot: u32,
    pub initprot: u32,
}

impl SegmentPlan {
    #[must_use]
    pub fn name_str(&self) -> &str {
        let end = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.name.len());
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }

    #[must_use]
    pub fn is_pagezero(&self) -> bool {
        self.name_str() == "__PAGEZERO"
    }
}

#[derive(Clone, Debug, Default)]
pub struct LoadPlan {
    pub entryoff: Option<u64>,
    pub entry_vmaddr: Option<u64>,
    pub segments: Vec<SegmentPlan>,
    pub dylibs: Vec<String>,
    pub rpaths: Vec<String>,
    pub has_chained_fixups: bool,
    pub has_dyld_info: bool,
    pub chained_fixups: Option<LinkeditDataCmd>,
    pub dyld_info: Option<DyldInfoCmd>,
    pub symtab: Option<SymtabCmd>,
    pub dysymtab: Option<DysymtabCmd>,
}

// ---------------------------------------------------------------------------
// Symbol binding
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SymbolBinding {
    pub name: String,
    pub source_dylib: Option<String>,
    pub address: Option<usize>,
    pub weak: bool,
}

// ---------------------------------------------------------------------------
// Chained fixups
// ---------------------------------------------------------------------------

pub const DYLD_CHAINED_IMPORT: u32 = 1;
pub const DYLD_CHAINED_IMPORT_ADDEND: u32 = 2;
pub const DYLD_CHAINED_IMPORT_ADDEND64: u32 = 3;

pub const DYLD_CHAINED_PTR_START_NONE: u16 = 0xFFFF;
pub const DYLD_CHAINED_PTR_START_MULTI: u16 = 0x8000;

pub const DYLD_CHAINED_PTR_64: u16 = 2;
pub const DYLD_CHAINED_PTR_64_OFFSET: u16 = 6;
pub const DYLD_CHAINED_PTR_ARM64E: u16 = 1;
pub const DYLD_CHAINED_PTR_ARM64E_OFFSET: u16 = 7;
pub const DYLD_CHAINED_PTR_ARM64E_USERLAND: u16 = 9;
pub const DYLD_CHAINED_PTR_ARM64E_USERLAND24: u16 = 12;

#[derive(Clone, Debug)]
pub struct ChainedImport {
    pub name: String,
    pub addend: i64,
    pub weak: bool,
    pub lib_ordinal: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ChainedFixupsHeader {
    pub fixups_version: u32,
    pub starts_offset: u32,
    pub imports_offset: u32,
    pub symbols_offset: u32,
    pub imports_count: u32,
    pub imports_format: u32,
    pub symbols_format: u32,
}

// ---------------------------------------------------------------------------
// Rebase / bind opcode constants
// ---------------------------------------------------------------------------

const REBASE_OPCODE_MASK: u8 = 0xF0;
const REBASE_IMMEDIATE_MASK: u8 = 0x0F;
const REBASE_OPCODE_DONE: u8 = 0x00;
const REBASE_OPCODE_SET_TYPE_IMM: u8 = 0x10;
const REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB: u8 = 0x20;
const REBASE_OPCODE_ADD_ADDR_ULEB: u8 = 0x30;
const REBASE_OPCODE_ADD_ADDR_IMM_SCALED: u8 = 0x40;
const REBASE_OPCODE_DO_REBASE_IMM_TIMES: u8 = 0x50;
const REBASE_OPCODE_DO_REBASE_ULEB_TIMES: u8 = 0x60;
const REBASE_OPCODE_DO_REBASE_ADD_ADDR_ULEB: u8 = 0x70;
const REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB: u8 = 0x80;
const REBASE_TYPE_POINTER: u8 = 1;

const BIND_OPCODE_MASK: u8 = 0xF0;
const BIND_IMMEDIATE_MASK: u8 = 0x0F;
const BIND_OPCODE_DONE: u8 = 0x00;
const BIND_OPCODE_SET_DYLIB_ORDINAL_IMM: u8 = 0x10;
const BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB: u8 = 0x20;
const BIND_OPCODE_SET_DYLIB_SPECIAL_IMM: u8 = 0x30;
const BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM: u8 = 0x40;
const BIND_OPCODE_SET_TYPE_IMM: u8 = 0x50;
const BIND_OPCODE_SET_ADDEND_SLEB: u8 = 0x60;
const BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB: u8 = 0x70;
const BIND_OPCODE_ADD_ADDR_ULEB: u8 = 0x80;
const BIND_OPCODE_DO_BIND: u8 = 0x90;
const BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB: u8 = 0xA0;
const BIND_OPCODE_DO_BIND_ADD_ADDR_IMM_SCALED: u8 = 0xB0;
const BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB: u8 = 0xC0;
const BIND_TYPE_POINTER: u8 = 1;
const BIND_SYMBOL_FLAGS_WEAK_IMPORT: u8 = 0x01;

const ARM_THREAD_STATE64: u32 = 6;
const ARM_THREAD_STATE64_COUNT: u32 = 68;

// ---------------------------------------------------------------------------
// Fat / thin slice selection
// ---------------------------------------------------------------------------

pub fn select_arm64_slice(bytes: &[u8]) -> Result<&[u8]> {
    if bytes.len() < 4 {
        return Err(Error::TruncatedFile);
    }
    let magic = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    match magic {
        goblin::mach::header::MH_MAGIC_64 | goblin::mach::header::MH_CIGAM_64 => Ok(bytes),
        fat::FAT_MAGIC | 0xBEBA_FECA => select_arm64_from_fat(bytes),
        _ => Err(Error::UnsupportedBinaryFormat),
    }
}

fn select_arm64_from_fat(bytes: &[u8]) -> Result<&[u8]> {
    let hdr_size = 8;
    if bytes.len() < hdr_size {
        return Err(Error::TruncatedFile);
    }
    let nfat = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let arch_size = 20; // sizeof(fat_arch)

    for i in 0..nfat {
        let off = hdr_size + i * arch_size;
        if off + arch_size > bytes.len() {
            return Err(Error::TruncatedFile);
        }
        let cputype = u32::from_be_bytes(bytes[off..off + 4].try_into().unwrap());
        if cputype != CPU_TYPE_ARM64 {
            continue;
        }
        let slice_off = u32::from_be_bytes(bytes[off + 8..off + 12].try_into().unwrap()) as usize;
        let slice_len = u32::from_be_bytes(bytes[off + 12..off + 16].try_into().unwrap()) as usize;
        let end = slice_off + slice_len;
        if end > bytes.len() {
            return Err(Error::TruncatedFile);
        }
        return Ok(&bytes[slice_off..end]);
    }
    Err(Error::Arm64SliceNotFound)
}

// ---------------------------------------------------------------------------
// Load plan construction
// ---------------------------------------------------------------------------

pub fn build_load_plan(bytes: &[u8]) -> Result<LoadPlan> {
    let macho = goblin::mach::MachO::parse(bytes, 0)?;

    if macho.header.cputype() != CPU_TYPE_ARM64 {
        return Err(Error::NotArm64);
    }

    let mut plan = LoadPlan::default();

    for lc in &macho.load_commands {
        match &lc.command {
            CommandVariant::Segment64(seg) => {
                plan.segments.push(SegmentPlan {
                    name: seg.segname,
                    vmaddr: seg.vmaddr,
                    vmsize: seg.vmsize,
                    fileoff: seg.fileoff,
                    filesize: seg.filesize,
                    maxprot: seg.maxprot,
                    initprot: seg.initprot,
                });
            }
            CommandVariant::Main(ep) => {
                plan.entryoff = Some(ep.entryoff);
            }
            CommandVariant::Unixthread(_ut) => {
                if plan.entryoff.is_none()
                    && plan.entry_vmaddr.is_none()
                    && let Some(pc) =
                        thread_entry_pc(&bytes[lc.offset..lc.offset + lc.command.cmdsize()])
                {
                    plan.entry_vmaddr = Some(pc);
                }
            }
            CommandVariant::LoadDylib(cmd)
            | CommandVariant::LoadWeakDylib(cmd)
            | CommandVariant::ReexportDylib(cmd)
            | CommandVariant::LazyLoadDylib(cmd) => {
                let name_off = cmd.dylib.name as usize;
                if let Some(name) =
                    load_command_string(bytes, lc.offset, lc.command.cmdsize(), name_off)
                {
                    plan.dylibs.push(String::from(name));
                }
            }
            CommandVariant::Rpath(cmd) => {
                let path_off = cmd.path as usize;
                if let Some(path) =
                    load_command_string(bytes, lc.offset, lc.command.cmdsize(), path_off)
                {
                    plan.rpaths.push(String::from(path));
                }
            }
            CommandVariant::DyldChainedFixups(cmd) => {
                plan.has_chained_fixups = true;
                plan.chained_fixups = Some(LinkeditDataCmd {
                    dataoff: cmd.dataoff,
                    datasize: cmd.datasize,
                });
            }
            CommandVariant::DyldInfo(cmd) | CommandVariant::DyldInfoOnly(cmd) => {
                plan.has_dyld_info = true;
                plan.dyld_info = Some(DyldInfoCmd {
                    rebase_off: cmd.rebase_off,
                    rebase_size: cmd.rebase_size,
                    bind_off: cmd.bind_off,
                    bind_size: cmd.bind_size,
                    weak_bind_off: cmd.weak_bind_off,
                    weak_bind_size: cmd.weak_bind_size,
                    lazy_bind_off: cmd.lazy_bind_off,
                    lazy_bind_size: cmd.lazy_bind_size,
                    export_off: cmd.export_off,
                    export_size: cmd.export_size,
                });
            }
            CommandVariant::Symtab(cmd) => {
                plan.symtab = Some(SymtabCmd {
                    symoff: cmd.symoff,
                    nsyms: cmd.nsyms,
                    stroff: cmd.stroff,
                    strsize: cmd.strsize,
                });
            }
            CommandVariant::Dysymtab(cmd) => {
                plan.dysymtab = Some(DysymtabCmd {
                    iundefsym: cmd.iundefsym,
                    nundefsym: cmd.nundefsym,
                });
            }
            _ => {}
        }
    }

    if plan.entryoff.is_none() && plan.entry_vmaddr.is_none() {
        return Err(Error::MissingEntryPoint);
    }
    Ok(plan)
}

fn thread_entry_pc(cmd_bytes: &[u8]) -> Option<u64> {
    let lc_size = 8; // load_command header
    if cmd_bytes.len() < lc_size + 8 {
        return None;
    }
    let mut off = lc_size;
    while off + 8 <= cmd_bytes.len() {
        let flavor = u32::from_le_bytes(cmd_bytes[off..off + 4].try_into().ok()?);
        let count = u32::from_le_bytes(cmd_bytes[off + 4..off + 8].try_into().ok()?);
        off += 8;
        let state_size = count as usize * 4;
        if off + state_size > cmd_bytes.len() {
            return None;
        }
        if flavor == ARM_THREAD_STATE64 && count >= ARM_THREAD_STATE64_COUNT && state_size >= 272 {
            // pc is at: x[29] + fp + lr + sp = 32 u64s = 256 bytes offset
            let pc_off = off + 256;
            let pc = u64::from_le_bytes(cmd_bytes[pc_off..pc_off + 8].try_into().ok()?);
            return Some(pc);
        }
        off += state_size;
    }
    None
}

fn load_command_string(
    bytes: &[u8],
    lc_off: usize,
    lc_size: usize,
    str_off: usize,
) -> Option<&str> {
    let name_off = lc_off + str_off;
    let cmd_end = lc_off + lc_size;
    if name_off >= cmd_end || cmd_end > bytes.len() {
        return None;
    }
    let region = &bytes[name_off..cmd_end];
    let end = region.iter().position(|&b| b == 0).unwrap_or(region.len());
    core::str::from_utf8(&region[..end]).ok()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn read_le_u16(bytes: &[u8], off: usize) -> Result<u16> {
    bytes
        .get(off..off + 2)
        .and_then(|s| s.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or(Error::TruncatedFile)
}

pub fn read_le_u32(bytes: &[u8], off: usize) -> Result<u32> {
    bytes
        .get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(Error::TruncatedFile)
}

pub fn read_le_u64(bytes: &[u8], off: usize) -> Result<u64> {
    bytes
        .get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(Error::TruncatedFile)
}

pub fn read_le_i32(bytes: &[u8], off: usize) -> Result<i32> {
    bytes
        .get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .map(i32::from_le_bytes)
        .ok_or(Error::TruncatedFile)
}

pub fn read_le_i64(bytes: &[u8], off: usize) -> Result<i64> {
    bytes
        .get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .map(i64::from_le_bytes)
        .ok_or(Error::TruncatedFile)
}

pub fn write_le_u64(bytes: &mut [u8], off: usize, value: u64) -> Result<()> {
    let dst = bytes.get_mut(off..off + 8).ok_or(Error::TruncatedFile)?;
    dst.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

pub fn read_uleb128(stream: &[u8], idx: &mut usize) -> Result<u64> {
    let mut shift: u32 = 0;
    let mut result: u64 = 0;
    while *idx < stream.len() {
        let b = stream[*idx];
        *idx += 1;
        result |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err(Error::InvalidLeb128);
        }
    }
    Err(Error::TruncatedDyldInfo)
}

pub fn read_sleb128(stream: &[u8], idx: &mut usize) -> Result<i64> {
    let mut shift: u32 = 0;
    let mut result: i64 = 0;
    let mut byte: u8 = 0;
    while *idx < stream.len() {
        byte = stream[*idx];
        *idx += 1;
        result |= i64::from(byte & 0x7F) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            break;
        }
        if shift >= 64 {
            return Err(Error::InvalidLeb128);
        }
    }
    if byte & 0x40 != 0 && shift < 64 {
        result |= -(1i64 << shift);
    }
    Ok(result)
}

pub fn read_cstring<'a>(stream: &'a [u8], idx: &mut usize) -> Result<&'a str> {
    let start = *idx;
    while *idx < stream.len() && stream[*idx] != 0 {
        *idx += 1;
    }
    if *idx >= stream.len() {
        return Err(Error::TruncatedDyldInfo);
    }
    let s = core::str::from_utf8(&stream[start..*idx]).map_err(|_| Error::TruncatedDyldInfo)?;
    *idx += 1; // skip null
    Ok(s)
}

pub fn symbol_from_pool(blob: &[u8], pool_off: usize, name_off: usize) -> Result<&str> {
    let start = pool_off + name_off;
    if start >= blob.len() {
        return Err(Error::InvalidChainedSymbols);
    }
    let region = &blob[start..];
    let end = region.iter().position(|&b| b == 0).unwrap_or(region.len());
    core::str::from_utf8(&region[..end]).map_err(|_| Error::InvalidChainedSymbols)
}

#[must_use]
pub fn install_name_for_ordinal(plan: &LoadPlan, dylib_ordinal: i32) -> Option<&str> {
    if dylib_ordinal <= 0 {
        return None;
    }
    let idx = (dylib_ordinal - 1) as usize;
    plan.dylibs.get(idx).map(String::as_str)
}

#[must_use]
pub fn entry_vmaddr_from_offset(entryoff: u64, segments: &[SegmentPlan]) -> Option<u64> {
    for seg in segments {
        if seg.is_pagezero() {
            continue;
        }
        if entryoff < seg.fileoff {
            continue;
        }
        let seg_end = seg.fileoff + seg.filesize;
        if entryoff >= seg_end {
            continue;
        }
        return Some(seg.vmaddr + (entryoff - seg.fileoff));
    }
    None
}

#[must_use]
pub fn min_mapped_vmaddr(segments: &[SegmentPlan]) -> Option<u64> {
    let mut min: Option<u64> = None;
    for seg in segments {
        if seg.is_pagezero() || seg.vmsize == 0 {
            continue;
        }
        min = Some(match min {
            Some(m) => core::cmp::min(m, seg.vmaddr),
            None => seg.vmaddr,
        });
    }
    min
}

pub fn segment_runtime_offset(
    seg: &SegmentPlan,
    seg_off: u64,
    plan: &LoadPlan,
    mapped_len: usize,
) -> Result<usize> {
    let min = min_mapped_vmaddr(&plan.segments).ok_or(Error::NoMappableSegments)?;
    let abs = seg.vmaddr + seg_off;
    if abs < min {
        return Err(Error::InvalidSegmentOffset);
    }
    let off = usize::try_from(abs - min).map_err(|_| Error::ImageTooLarge)?;
    if off + size_of::<u64>() > mapped_len {
        return Err(Error::InvalidSegmentOffset);
    }
    Ok(off)
}

// ---------------------------------------------------------------------------
// Undefined symbol extraction (symtab/dysymtab)
// ---------------------------------------------------------------------------

pub struct UndefSymbol<'a> {
    pub name: &'a str,
    pub weak: bool,
    pub dylib_ordinal: i32,
}

pub fn iter_undefined_symbols<'a>(
    bytes: &'a [u8],
    plan: &'a LoadPlan,
) -> Result<Vec<UndefSymbol<'a>>> {
    let symtab = match plan.symtab {
        Some(s) => s,
        None => return Ok(Vec::new()),
    };
    let dysymtab = match plan.dysymtab {
        Some(d) => d,
        None => return Ok(Vec::new()),
    };

    let sym_size = 16usize; // sizeof(nlist_64)
    let symoff = symtab.symoff as usize;
    let nsyms = symtab.nsyms as usize;
    let symtab_end = symoff + nsyms * sym_size;
    if symtab_end > bytes.len() {
        return Err(Error::TruncatedSymtab);
    }

    let stroff = symtab.stroff as usize;
    let strsize = symtab.strsize as usize;
    let strtab_end = stroff + strsize;
    if strtab_end > bytes.len() {
        return Err(Error::TruncatedStrtab);
    }
    let strtab = &bytes[stroff..strtab_end];

    let iundef = dysymtab.iundefsym as usize;
    let nundef = dysymtab.nundefsym as usize;
    if iundef + nundef > nsyms {
        return Err(Error::InvalidUndefinedSymbolRange);
    }

    let mut out = Vec::with_capacity(nundef);
    for sym_idx in iundef..(iundef + nundef) {
        let off = symoff + sym_idx * sym_size;
        let n_strx = read_le_u32(bytes, off)? as usize;
        let n_type = bytes[off + 4];
        let n_desc = read_le_u16(bytes, off + 10)?;

        // Skip stabs
        if n_type & 0xE0 != 0 {
            continue;
        }
        // Only N_UNDF
        if n_type & 0x0E != 0 {
            continue;
        }

        let name = symbol_name_from_strtab(strtab, n_strx)?;
        if name.is_empty() {
            continue;
        }

        let weak = n_desc & 0x0040 != 0; // N_WEAK_REF
        let ord_byte = (n_desc >> 8) as u8;
        let dylib_ordinal: i32 = if ord_byte == 0 || ord_byte == 0xFE || ord_byte == 0xFF {
            0
        } else {
            i32::from(ord_byte)
        };

        out.push(UndefSymbol {
            name,
            weak,
            dylib_ordinal,
        });
    }
    Ok(out)
}

fn symbol_name_from_strtab(strtab: &[u8], n_strx: usize) -> Result<&str> {
    if n_strx >= strtab.len() {
        return Err(Error::InvalidSymbolStringOffset);
    }
    let region = &strtab[n_strx..];
    let end = region.iter().position(|&b| b == 0).unwrap_or(region.len());
    core::str::from_utf8(&region[..end]).map_err(|_| Error::InvalidSymbolStringOffset)
}

// ---------------------------------------------------------------------------
// Export trie
// ---------------------------------------------------------------------------

pub fn find_export_trie(bytes: &[u8]) -> Result<&[u8]> {
    let macho = goblin::mach::MachO::parse(bytes, 0)?;
    let mut export_off: usize = 0;
    let mut export_size: usize = 0;

    for lc in &macho.load_commands {
        match &lc.command {
            CommandVariant::DyldExportsTrie(cmd) => {
                export_off = cmd.dataoff as usize;
                export_size = cmd.datasize as usize;
            }
            CommandVariant::DyldInfo(cmd) | CommandVariant::DyldInfoOnly(cmd)
                if export_off == 0 && cmd.export_size != 0 =>
            {
                export_off = cmd.export_off as usize;
                export_size = cmd.export_size as usize;
            }
            _ => {}
        }
    }

    if export_off == 0 || export_size == 0 {
        return Err(Error::NoExportTrie);
    }
    if export_off + export_size > bytes.len() {
        return Err(Error::TruncatedFile);
    }
    Ok(&bytes[export_off..export_off + export_size])
}

pub fn parse_export_trie_symbols(trie: &[u8]) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    let mut prefix = Vec::new();
    let mut visiting = BTreeSet::new();
    walk_export_node(0, trie, &mut prefix, &mut out, &mut visiting)?;
    Ok(out)
}

fn walk_export_node(
    node_off: usize,
    trie: &[u8],
    prefix: &mut Vec<u8>,
    out: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<usize>,
) -> Result<()> {
    if node_off >= trie.len() {
        return Err(Error::InvalidExportTrie);
    }
    if !visiting.insert(node_off) {
        return Ok(());
    }

    let mut idx = node_off;
    let terminal_size = read_uleb128(trie, &mut idx)? as usize;
    let terminal_end = idx + terminal_size;
    if terminal_end > trie.len() {
        return Err(Error::InvalidExportTrie);
    }
    if terminal_size != 0
        && let Ok(s) = core::str::from_utf8(prefix)
    {
        out.insert(String::from(s));
    }
    idx = terminal_end;
    if idx >= trie.len() {
        visiting.remove(&node_off);
        return Err(Error::InvalidExportTrie);
    }

    let child_count = trie[idx] as usize;
    idx += 1;
    for _ in 0..child_count {
        let edge_start = idx;
        while idx < trie.len() && trie[idx] != 0 {
            idx += 1;
        }
        if idx >= trie.len() {
            visiting.remove(&node_off);
            return Err(Error::InvalidExportTrie);
        }
        let edge = &trie[edge_start..idx];
        idx += 1; // null
        let child_off = read_uleb128(trie, &mut idx)? as usize;
        let old_len = prefix.len();
        prefix.extend_from_slice(edge);
        walk_export_node(child_off, trie, prefix, out, visiting)?;
        prefix.truncate(old_len);
    }

    visiting.remove(&node_off);
    Ok(())
}

// ---------------------------------------------------------------------------
// Chained imports parsing
// ---------------------------------------------------------------------------

pub fn parse_chained_fixups_header(blob: &[u8]) -> Result<ChainedFixupsHeader> {
    if blob.len() < size_of::<ChainedFixupsHeader>() {
        return Err(Error::InvalidChainedFixups);
    }
    Ok(ChainedFixupsHeader {
        fixups_version: read_le_u32(blob, 0)?,
        starts_offset: read_le_u32(blob, 4)?,
        imports_offset: read_le_u32(blob, 8)?,
        symbols_offset: read_le_u32(blob, 12)?,
        imports_count: read_le_u32(blob, 16)?,
        imports_format: read_le_u32(blob, 20)?,
        symbols_format: read_le_u32(blob, 24)?,
    })
}

pub fn parse_chained_imports(blob: &[u8], hdr: &ChainedFixupsHeader) -> Result<Vec<ChainedImport>> {
    let count = hdr.imports_count as usize;
    let base = hdr.imports_offset as usize;
    if base > blob.len() {
        return Err(Error::InvalidChainedImports);
    }
    let symbol_base = hdr.symbols_offset as usize;
    if symbol_base > blob.len() {
        return Err(Error::InvalidChainedSymbols);
    }

    let mut out = Vec::with_capacity(count);

    match hdr.imports_format {
        DYLD_CHAINED_IMPORT => {
            let rec_size = 4;
            if base + count * rec_size > blob.len() {
                return Err(Error::InvalidChainedImports);
            }
            for i in 0..count {
                let raw = read_le_u32(blob, base + i * rec_size)?;
                let lib_ordinal = (raw & 0xFF) as i32;
                let weak = (raw >> 8) & 1 != 0;
                let name_off = ((raw >> 9) & 0x7F_FFFF) as usize;
                out.push(ChainedImport {
                    name: String::from(symbol_from_pool(blob, symbol_base, name_off)?),
                    weak,
                    addend: 0,
                    lib_ordinal,
                });
            }
        }
        DYLD_CHAINED_IMPORT_ADDEND => {
            let rec_size = 8;
            if base + count * rec_size > blob.len() {
                return Err(Error::InvalidChainedImports);
            }
            for i in 0..count {
                let raw = read_le_u32(blob, base + i * rec_size)?;
                let addend_raw = read_le_i32(blob, base + i * rec_size + 4)?;
                let lib_ordinal = (raw & 0xFF) as i32;
                let weak = (raw >> 8) & 1 != 0;
                let name_off = ((raw >> 9) & 0x7F_FFFF) as usize;
                out.push(ChainedImport {
                    name: String::from(symbol_from_pool(blob, symbol_base, name_off)?),
                    weak,
                    addend: i64::from(addend_raw),
                    lib_ordinal,
                });
            }
        }
        DYLD_CHAINED_IMPORT_ADDEND64 => {
            let rec_size = 16;
            if base + count * rec_size > blob.len() {
                return Err(Error::InvalidChainedImports);
            }
            for i in 0..count {
                let raw = read_le_u64(blob, base + i * rec_size)?;
                let addend_raw = read_le_i64(blob, base + i * rec_size + 8)?;
                let lib_ordinal = (raw & 0xFFFF) as i32;
                let weak = (raw >> 16) & 1 != 0;
                let name_off = ((raw >> 32) & 0xFFFF_FFFF) as usize;
                out.push(ChainedImport {
                    name: String::from(symbol_from_pool(blob, symbol_base, name_off)?),
                    weak,
                    addend: addend_raw,
                    lib_ordinal,
                });
            }
        }
        _ => return Err(Error::UnsupportedChainedImportsFormat),
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Chained fixup application
// ---------------------------------------------------------------------------

pub fn apply_chained_fixups(
    bytes: &[u8],
    plan: &LoadPlan,
    mapped: &mut [u8],
    min_vmaddr: u64,
    resolve: &dyn Fn(&str, Option<&str>) -> Option<usize>,
) -> Result<()> {
    let fix_cmd = match plan.chained_fixups {
        Some(c) => c,
        None => return Ok(()),
    };

    let fixoff = fix_cmd.dataoff as usize;
    let fixsize = fix_cmd.datasize as usize;
    if fixoff + fixsize > bytes.len() {
        return Err(Error::TruncatedChainedFixups);
    }
    let blob = &bytes[fixoff..fixoff + fixsize];
    let hdr = parse_chained_fixups_header(blob)?;
    if hdr.fixups_version != 0 {
        return Err(Error::UnsupportedFixupsVersion);
    }
    if hdr.symbols_format != 0 {
        return Err(Error::UnsupportedFixupsSymbolsFormat);
    }

    let imports = parse_chained_imports(blob, &hdr)?;

    let starts_off = hdr.starts_offset as usize;
    if starts_off + 4 > blob.len() {
        return Err(Error::InvalidChainedStarts);
    }
    let seg_count = read_le_u32(blob, starts_off)? as usize;
    let seg_table_off = starts_off + 4;
    if seg_table_off + seg_count * 4 > blob.len() {
        return Err(Error::InvalidChainedStarts);
    }

    for seg_index in 0..seg_count {
        let info_rel = read_le_u32(blob, seg_table_off + seg_index * 4)? as usize;
        if info_rel == 0 {
            continue;
        }
        if seg_index >= plan.segments.len() {
            continue;
        }
        apply_segment_chains(
            plan,
            blob,
            starts_off + info_rel,
            &imports,
            mapped,
            min_vmaddr,
            resolve,
        )?;
    }
    Ok(())
}

fn apply_segment_chains(
    plan: &LoadPlan,
    blob: &[u8],
    seg_info_off: usize,
    imports: &[ChainedImport],
    mapped: &mut [u8],
    min_vmaddr: u64,
    resolve: &dyn Fn(&str, Option<&str>) -> Option<usize>,
) -> Result<()> {
    if seg_info_off + 24 > blob.len() {
        return Err(Error::InvalidChainedStarts);
    }
    let size = read_le_u32(blob, seg_info_off)? as usize;
    if size < 24 || seg_info_off + size > blob.len() {
        return Err(Error::InvalidChainedStarts);
    }
    let page_size = read_le_u16(blob, seg_info_off + 4)?;
    let ptr_format = read_le_u16(blob, seg_info_off + 6)?;
    let segment_offset = read_le_u64(blob, seg_info_off + 8)?;
    let _max_valid_pointer = read_le_u32(blob, seg_info_off + 16)?;
    let page_count = read_le_u16(blob, seg_info_off + 20)? as usize;
    let page_start_off = seg_info_off + 22;
    if page_start_off + page_count * 2 > seg_info_off + size {
        return Err(Error::InvalidChainedStarts);
    }

    let is_64 = ptr_format == DYLD_CHAINED_PTR_64 || ptr_format == DYLD_CHAINED_PTR_64_OFFSET;
    let is_arm64e = ptr_format == DYLD_CHAINED_PTR_ARM64E
        || ptr_format == DYLD_CHAINED_PTR_ARM64E_OFFSET
        || ptr_format == DYLD_CHAINED_PTR_ARM64E_USERLAND
        || ptr_format == DYLD_CHAINED_PTR_ARM64E_USERLAND24;
    if !is_64 && !is_arm64e {
        return Err(Error::UnsupportedChainedPointerFormat);
    }

    let mapped_base = mapped.as_ptr() as u64;
    let slide = mapped_base.wrapping_sub(min_vmaddr);

    for page_idx in 0..page_count {
        let start = read_le_u16(blob, page_start_off + page_idx * 2)?;
        if start == DYLD_CHAINED_PTR_START_NONE {
            continue;
        }
        if start & DYLD_CHAINED_PTR_START_MULTI != 0 {
            let mut multi_idx = (start & !DYLD_CHAINED_PTR_START_MULTI) as usize;
            loop {
                let multi_off = seg_info_off + multi_idx * 2;
                let entry = read_le_u16(blob, multi_off)?;
                let is_last = entry & DYLD_CHAINED_PTR_START_MULTI != 0;
                let sub_start = entry & !DYLD_CHAINED_PTR_START_MULTI;
                process_chain_at_start(
                    plan,
                    sub_start,
                    page_idx,
                    page_size,
                    segment_offset,
                    is_64,
                    ptr_format,
                    imports,
                    mapped,
                    mapped_base,
                    slide,
                    resolve,
                )?;
                if is_last {
                    break;
                }
                multi_idx += 1;
            }
        } else {
            process_chain_at_start(
                plan,
                start,
                page_idx,
                page_size,
                segment_offset,
                is_64,
                ptr_format,
                imports,
                mapped,
                mapped_base,
                slide,
                resolve,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn process_chain_at_start(
    plan: &LoadPlan,
    start: u16,
    page_idx: usize,
    page_size: u16,
    segment_offset: u64,
    is_64: bool,
    ptr_format: u16,
    imports: &[ChainedImport],
    mapped: &mut [u8],
    mapped_base: u64,
    slide: u64,
    resolve: &dyn Fn(&str, Option<&str>) -> Option<usize>,
) -> Result<()> {
    let mut chain_off_u64 =
        segment_offset + u64::from(page_idx as u32) * u64::from(page_size) + u64::from(start);
    loop {
        let chain_off = usize::try_from(chain_off_u64).map_err(|_| Error::ImageTooLarge)?;
        if chain_off + 8 > mapped.len() {
            return Err(Error::InvalidChainedPointerLocation);
        }
        let raw = read_le_u64(mapped, chain_off)?;

        if is_64 {
            let bind = (raw >> 63) != 0;
            let next = (raw >> 51) & 0xFFF;

            if bind {
                let ordinal = (raw & 0x00FF_FFFF) as usize;
                let addend8 = ((raw >> 24) & 0xFF) as u8 as i8;
                if ordinal >= imports.len() {
                    return Err(Error::InvalidChainedImportOrdinal);
                }
                let imp = &imports[ordinal];
                let preferred = install_name_for_ordinal(plan, imp.lib_ordinal);
                let sym_addr = resolve(&imp.name, preferred);
                if sym_addr.is_none() && !imp.weak {
                    return Err(Error::UnresolvedChainedImport(imp.name.clone()));
                }
                let base_addr = sym_addr.unwrap_or(0) as i128;
                let total = base_addr + i128::from(imp.addend) + i128::from(addend8);
                write_le_u64(mapped, chain_off, total as u64)?;
            } else {
                let target = raw & ((1u64 << 36) - 1);
                let high8 = (raw >> 36) & 0xFF;
                let out_ptr = if ptr_format == DYLD_CHAINED_PTR_64_OFFSET {
                    mapped_base + target
                } else {
                    ((high8 << 56) | target) + slide
                };
                write_le_u64(mapped, chain_off, out_ptr)?;
            }

            if next == 0 {
                break;
            }
            chain_off_u64 += next * 4;
        } else {
            let auth = (raw >> 63) != 0;
            let bind = ((raw >> 62) & 1) != 0;
            let next = (raw >> 51) & 0x7FF;

            if bind && !auth {
                let ordinal_bits: u32 = if ptr_format == DYLD_CHAINED_PTR_ARM64E_USERLAND24 {
                    24
                } else {
                    16
                };
                let ordinal_mask = (1u64 << ordinal_bits) - 1;
                let ordinal = (raw & ordinal_mask) as usize;
                if ordinal >= imports.len() {
                    return Err(Error::InvalidChainedImportOrdinal);
                }
                let addend_bits = (raw >> 32) & 0x7_FFFF;
                let addend19 = i64::from(((addend_bits as u32) << 13) as i32 >> 13);
                let imp = &imports[ordinal];
                let preferred = install_name_for_ordinal(plan, imp.lib_ordinal);
                let sym_addr = resolve(&imp.name, preferred);
                if sym_addr.is_none() && !imp.weak {
                    return Err(Error::UnresolvedChainedImport(imp.name.clone()));
                }
                let base_addr = sym_addr.unwrap_or(0) as i128;
                let total = base_addr + i128::from(imp.addend) + i128::from(addend19);
                write_le_u64(mapped, chain_off, total as u64)?;
            } else if !bind && !auth {
                let target = raw & ((1u64 << 43) - 1);
                let is_offset = ptr_format == DYLD_CHAINED_PTR_ARM64E_OFFSET
                    || ptr_format == DYLD_CHAINED_PTR_ARM64E_USERLAND
                    || ptr_format == DYLD_CHAINED_PTR_ARM64E_USERLAND24;
                let out_ptr = if is_offset {
                    mapped_base + target
                } else {
                    target + slide
                };
                write_le_u64(mapped, chain_off, out_ptr)?;
            } else if bind && auth {
                let ordinal_bits: u32 = if ptr_format == DYLD_CHAINED_PTR_ARM64E_USERLAND24 {
                    24
                } else {
                    16
                };
                let ordinal_mask = (1u64 << ordinal_bits) - 1;
                let ordinal = (raw & ordinal_mask) as usize;
                if ordinal >= imports.len() {
                    return Err(Error::InvalidChainedImportOrdinal);
                }
                let _diversity = ((raw >> 32) & 0xFFFF) as u16;
                let _addr_div = ((raw >> 48) & 1) != 0;
                let _key = ((raw >> 49) & 3) as u8;
                let imp = &imports[ordinal];
                let preferred = install_name_for_ordinal(plan, imp.lib_ordinal);
                let sym_addr = resolve(&imp.name, preferred);
                if sym_addr.is_none() && !imp.weak {
                    return Err(Error::UnresolvedChainedImport(imp.name.clone()));
                }
                let base_addr = sym_addr.unwrap_or(0) as i128;
                let unsigned_ptr = (base_addr + i128::from(imp.addend)) as u64;
                // PAC signing would happen here on real hardware; for now store unsigned
                write_le_u64(mapped, chain_off, unsigned_ptr)?;
            } else {
                // !bind && auth (rebase with auth)
                let target = raw & 0xFFFF_FFFF;
                let _diversity = ((raw >> 32) & 0xFFFF) as u16;
                let _addr_div = ((raw >> 48) & 1) != 0;
                let _key = ((raw >> 49) & 3) as u8;
                let is_offset = ptr_format == DYLD_CHAINED_PTR_ARM64E_OFFSET
                    || ptr_format == DYLD_CHAINED_PTR_ARM64E_USERLAND
                    || ptr_format == DYLD_CHAINED_PTR_ARM64E_USERLAND24;
                let unsigned_ptr = if is_offset {
                    mapped_base + target
                } else {
                    target + slide
                };
                write_le_u64(mapped, chain_off, unsigned_ptr)?;
            }

            if next == 0 {
                break;
            }
            chain_off_u64 += next * 8;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Dyld info fixup application (rebase + bind opcodes)
// ---------------------------------------------------------------------------

pub fn apply_dyld_info_fixups(
    plan: &LoadPlan,
    mapped: &mut [u8],
    min_vmaddr: u64,
    bytes: &[u8],
    resolve: &dyn Fn(&str, Option<&str>) -> Option<usize>,
) -> Result<()> {
    let info = match plan.dyld_info {
        Some(i) => i,
        None => return Ok(()),
    };

    if info.rebase_size != 0 {
        run_rebase_opcodes(
            plan,
            mapped,
            min_vmaddr,
            bytes,
            info.rebase_off,
            info.rebase_size,
        )?;
    }
    if info.bind_size != 0 {
        run_bind_opcodes(
            plan,
            mapped,
            min_vmaddr,
            bytes,
            info.bind_off,
            info.bind_size,
            resolve,
            BindStreamMode::Regular,
        )?;
    }
    if info.weak_bind_size != 0 {
        run_bind_opcodes(
            plan,
            mapped,
            min_vmaddr,
            bytes,
            info.weak_bind_off,
            info.weak_bind_size,
            resolve,
            BindStreamMode::Weak,
        )?;
    }
    if info.lazy_bind_size != 0 {
        run_bind_opcodes(
            plan,
            mapped,
            min_vmaddr,
            bytes,
            info.lazy_bind_off,
            info.lazy_bind_size,
            resolve,
            BindStreamMode::Lazy,
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BindStreamMode {
    Regular,
    Weak,
    Lazy,
}

fn run_rebase_opcodes(
    plan: &LoadPlan,
    mapped: &mut [u8],
    min_vmaddr: u64,
    bytes: &[u8],
    off: u32,
    size: u32,
) -> Result<()> {
    let start = off as usize;
    let end = start + size as usize;
    if end > bytes.len() {
        return Err(Error::TruncatedDyldInfo);
    }
    let stream = &bytes[start..end];
    let mapped_base = mapped.as_ptr() as u64;
    let slide = mapped_base.wrapping_sub(min_vmaddr);

    let mut idx = 0usize;
    let mut seg_idx = 0usize;
    let mut seg_off = 0u64;
    let mut rebase_type = REBASE_TYPE_POINTER;

    while idx < stream.len() {
        let op = stream[idx];
        idx += 1;
        let opcode = op & REBASE_OPCODE_MASK;
        let imm = op & REBASE_IMMEDIATE_MASK;
        match opcode {
            REBASE_OPCODE_DONE => break,
            REBASE_OPCODE_SET_TYPE_IMM => rebase_type = imm,
            REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB => {
                seg_idx = imm as usize;
                seg_off = read_uleb128(stream, &mut idx)?;
            }
            REBASE_OPCODE_ADD_ADDR_ULEB => {
                seg_off += read_uleb128(stream, &mut idx)?;
            }
            REBASE_OPCODE_ADD_ADDR_IMM_SCALED => {
                seg_off += u64::from(imm) * 8;
            }
            REBASE_OPCODE_DO_REBASE_IMM_TIMES => {
                for _ in 0..imm {
                    write_rebase_pointer(plan, mapped, seg_idx, seg_off, slide)?;
                    seg_off += 8;
                }
            }
            REBASE_OPCODE_DO_REBASE_ULEB_TIMES => {
                let count = read_uleb128(stream, &mut idx)?;
                for _ in 0..count {
                    write_rebase_pointer(plan, mapped, seg_idx, seg_off, slide)?;
                    seg_off += 8;
                }
            }
            REBASE_OPCODE_DO_REBASE_ADD_ADDR_ULEB => {
                write_rebase_pointer(plan, mapped, seg_idx, seg_off, slide)?;
                seg_off += 8 + read_uleb128(stream, &mut idx)?;
            }
            REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB => {
                let count = read_uleb128(stream, &mut idx)?;
                let skip = read_uleb128(stream, &mut idx)?;
                for _ in 0..count {
                    write_rebase_pointer(plan, mapped, seg_idx, seg_off, slide)?;
                    seg_off += 8 + skip;
                }
            }
            _ => return Err(Error::UnsupportedRebaseOpcode),
        }
        if rebase_type != REBASE_TYPE_POINTER {
            return Err(Error::UnsupportedRebaseType);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_bind_opcodes(
    plan: &LoadPlan,
    mapped: &mut [u8],
    _min_vmaddr: u64,
    bytes: &[u8],
    off: u32,
    size: u32,
    resolve: &dyn Fn(&str, Option<&str>) -> Option<usize>,
    mode: BindStreamMode,
) -> Result<()> {
    let start = off as usize;
    let end = start + size as usize;
    if end > bytes.len() {
        return Err(Error::TruncatedDyldInfo);
    }
    let stream = &bytes[start..end];

    let mut idx = 0usize;
    let mut seg_idx = 0usize;
    let mut seg_off = 0u64;
    let mut bind_type = BIND_TYPE_POINTER;
    let mut addend: i64 = 0;
    let mut symbol_name: &str = "";
    let mut symbol_flags: u8 = 0;
    let mut dylib_ordinal: i32 = 0;

    while idx < stream.len() {
        let op = stream[idx];
        idx += 1;
        let opcode = op & BIND_OPCODE_MASK;
        let imm = op & BIND_IMMEDIATE_MASK;
        match opcode {
            BIND_OPCODE_DONE => {
                if mode == BindStreamMode::Regular {
                    break;
                }
                addend = 0;
                symbol_name = "";
                symbol_flags = 0;
                bind_type = BIND_TYPE_POINTER;
                dylib_ordinal = 0;
                continue;
            }
            BIND_OPCODE_SET_DYLIB_ORDINAL_IMM => dylib_ordinal = i32::from(imm),
            BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB => {
                dylib_ordinal = read_uleb128(stream, &mut idx)? as i32;
            }
            BIND_OPCODE_SET_DYLIB_SPECIAL_IMM => {
                dylib_ordinal = if imm == 0 {
                    0
                } else {
                    i32::from((imm | 0xF0) as i8)
                };
            }
            BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM => {
                symbol_flags = imm;
                symbol_name = read_cstring(stream, &mut idx)?;
            }
            BIND_OPCODE_SET_TYPE_IMM => bind_type = imm,
            BIND_OPCODE_SET_ADDEND_SLEB => addend = read_sleb128(stream, &mut idx)?,
            BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB => {
                seg_idx = imm as usize;
                seg_off = read_uleb128(stream, &mut idx)?;
            }
            BIND_OPCODE_ADD_ADDR_ULEB => {
                seg_off += read_uleb128(stream, &mut idx)?;
            }
            BIND_OPCODE_DO_BIND => {
                write_bind_pointer(
                    plan,
                    mapped,
                    seg_idx,
                    seg_off,
                    resolve,
                    symbol_name,
                    addend,
                    symbol_flags,
                    mode == BindStreamMode::Weak,
                    dylib_ordinal,
                )?;
                seg_off += 8;
            }
            BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB => {
                write_bind_pointer(
                    plan,
                    mapped,
                    seg_idx,
                    seg_off,
                    resolve,
                    symbol_name,
                    addend,
                    symbol_flags,
                    mode == BindStreamMode::Weak,
                    dylib_ordinal,
                )?;
                seg_off += 8 + read_uleb128(stream, &mut idx)?;
            }
            BIND_OPCODE_DO_BIND_ADD_ADDR_IMM_SCALED => {
                write_bind_pointer(
                    plan,
                    mapped,
                    seg_idx,
                    seg_off,
                    resolve,
                    symbol_name,
                    addend,
                    symbol_flags,
                    mode == BindStreamMode::Weak,
                    dylib_ordinal,
                )?;
                seg_off += 8 + u64::from(imm) * 8;
            }
            BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB => {
                let count = read_uleb128(stream, &mut idx)?;
                let skip = read_uleb128(stream, &mut idx)?;
                for _ in 0..count {
                    write_bind_pointer(
                        plan,
                        mapped,
                        seg_idx,
                        seg_off,
                        resolve,
                        symbol_name,
                        addend,
                        symbol_flags,
                        mode == BindStreamMode::Weak,
                        dylib_ordinal,
                    )?;
                    seg_off += 8 + skip;
                }
            }
            _ => return Err(Error::UnsupportedBindOpcode),
        }
        if bind_type != BIND_TYPE_POINTER {
            return Err(Error::UnsupportedBindType);
        }
    }
    Ok(())
}

fn write_rebase_pointer(
    plan: &LoadPlan,
    mapped: &mut [u8],
    seg_idx: usize,
    seg_off: u64,
    slide: u64,
) -> Result<()> {
    if seg_idx >= plan.segments.len() {
        return Err(Error::InvalidSegmentIndex);
    }
    let seg = &plan.segments[seg_idx];
    let loc = segment_runtime_offset(seg, seg_off, plan, mapped.len())?;
    let current = read_le_u64(mapped, loc)?;
    write_le_u64(mapped, loc, current.wrapping_add(slide))
}

#[allow(clippy::too_many_arguments)]
fn write_bind_pointer(
    plan: &LoadPlan,
    mapped: &mut [u8],
    seg_idx: usize,
    seg_off: u64,
    resolve: &dyn Fn(&str, Option<&str>) -> Option<usize>,
    symbol_name: &str,
    addend: i64,
    symbol_flags: u8,
    allow_weak_missing: bool,
    dylib_ordinal: i32,
) -> Result<()> {
    if seg_idx >= plan.segments.len() {
        return Err(Error::InvalidSegmentIndex);
    }
    let seg = &plan.segments[seg_idx];
    let loc = segment_runtime_offset(seg, seg_off, plan, mapped.len())?;

    let preferred = install_name_for_ordinal(plan, dylib_ordinal);
    let sym_addr = resolve(symbol_name, preferred);
    if sym_addr.is_none() {
        let weak_import = symbol_flags & BIND_SYMBOL_FLAGS_WEAK_IMPORT != 0;
        if allow_weak_missing || weak_import {
            return write_le_u64(mapped, loc, 0);
        }
        return Err(Error::UnresolvedBindSymbol(String::from(symbol_name)));
    }
    let ptr_val = (sym_addr.unwrap() as i128 + i128::from(addend)) as u64;
    write_le_u64(mapped, loc, ptr_val)
}

// ---------------------------------------------------------------------------
// Shared cache parsing
// ---------------------------------------------------------------------------

pub fn parse_shared_cache_image_paths(bytes: &[u8]) -> Result<BTreeSet<String>> {
    if bytes.len() < 32 {
        return Err(Error::TruncatedFile);
    }
    if !bytes[..6].starts_with(b"dyld_v") {
        return Err(Error::UnsupportedBinaryFormat);
    }
    let images_off = read_le_u32(bytes, 24)? as usize;
    let images_count = read_le_u32(bytes, 28)? as usize;
    let image_info_size = 32usize;
    let table_end = images_off + images_count * image_info_size;
    if table_end > bytes.len() {
        return Err(Error::TruncatedFile);
    }

    let mut paths = BTreeSet::new();
    for i in 0..images_count {
        let rec = images_off + i * image_info_size;
        let path_off = read_le_u32(bytes, rec + 24)? as usize;
        if path_off >= bytes.len() {
            continue;
        }
        let region = &bytes[path_off..];
        let end = region.iter().position(|&b| b == 0).unwrap_or(region.len());
        if end == 0 {
            continue;
        }
        if let Ok(s) = core::str::from_utf8(&region[..end]) {
            paths.insert(String::from(s));
        }
    }
    Ok(paths)
}

// ---------------------------------------------------------------------------
// Collect image imports (dylibs + rpaths from a dependent image)
// ---------------------------------------------------------------------------

pub fn collect_image_imports(bytes: &[u8]) -> Result<(Vec<String>, Vec<String>)> {
    let macho = goblin::mach::MachO::parse(bytes, 0)?;
    if macho.header.cputype() != CPU_TYPE_ARM64 {
        return Err(Error::NotArm64);
    }

    let mut dylibs = Vec::new();
    let mut rpaths = Vec::new();

    for lc in &macho.load_commands {
        match &lc.command {
            CommandVariant::LoadDylib(cmd)
            | CommandVariant::LoadWeakDylib(cmd)
            | CommandVariant::ReexportDylib(cmd)
            | CommandVariant::LazyLoadDylib(cmd) => {
                let name_off = cmd.dylib.name as usize;
                if let Some(name) =
                    load_command_string(bytes, lc.offset, lc.command.cmdsize(), name_off)
                {
                    dylibs.push(String::from(name));
                }
            }
            CommandVariant::Rpath(cmd) => {
                let path_off = cmd.path as usize;
                if let Some(path) =
                    load_command_string(bytes, lc.offset, lc.command.cmdsize(), path_off)
                {
                    rpaths.push(String::from(path));
                }
            }
            _ => {}
        }
    }
    Ok((dylibs, rpaths))
}
