//! Read a Windows executable's FileVersion, including Proton installs.

use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_ptr: u32,
    raw_size: u32,
}

/// FileVersion from the main game executable, or nothing when it cannot be read.
pub fn detect_install_version(install_path: &Path) -> Option<String> {
    let exe = pick_main_exe(install_path)?;
    let bytes = fs::read(&exe).ok()?;
    file_version(&bytes)
}

fn pick_main_exe(install_path: &Path) -> Option<PathBuf> {
    if !install_path.is_dir() {
        return None;
    }
    let folder = install_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut best: Option<(i32, u64, PathBuf)> = None;
    for entry in WalkDir::new(install_path)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(rank) = exe_rank(entry.path(), &folder) else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let replace = match &best {
            None => true,
            Some((best_rank, best_size, _)) => {
                rank > *best_rank || (rank == *best_rank && size > *best_size)
            }
        };
        if replace {
            best = Some((rank, size, entry.path().to_path_buf()));
        }
    }
    best.map(|(_, _, path)| path)
}

fn exe_rank(path: &Path, folder: &str) -> Option<i32> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if !name.ends_with(".exe") {
        return None;
    }
    if name.contains("unins")
        || name.contains("setup")
        || name.contains("redist")
        || name.contains("crash")
    {
        return None;
    }
    if name.contains("launcher") && !name.contains("shipping") {
        return None;
    }
    let stem = name.trim_end_matches(".exe");
    let mut rank = 1;
    if stem.contains("shipping") {
        rank += 100;
    }
    let folder_compact: String = folder
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect();
    let stem_compact: String = stem.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if !folder_compact.is_empty()
        && (stem_compact.contains(&folder_compact) || folder_compact.contains(&stem_compact))
    {
        rank += 50;
    }
    Some(rank)
}

pub(crate) fn file_version(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return None;
    }
    let e_lfanew = read_u32(bytes, 0x3C)? as usize;
    if bytes.len() < e_lfanew.saturating_add(24) || &bytes[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        return None;
    }
    let coff = e_lfanew + 4;
    let num_sections = read_u16(bytes, coff + 2)? as usize;
    let size_opt = read_u16(bytes, coff + 16)? as usize;
    let opt = coff + 20;
    if size_opt < 120 || bytes.len() < opt.saturating_add(size_opt) {
        return None;
    }
    let magic = read_u16(bytes, opt)?;
    let (num_rva_off, dir_off) = match magic {
        0x10B => (92usize, 96usize),
        0x20B => (108usize, 112usize),
        _ => return None,
    };
    if size_opt < dir_off + 8 * 3 {
        return None;
    }
    let num_rva = read_u32(bytes, opt + num_rva_off)? as usize;
    if num_rva <= 2 {
        return None;
    }
    let res_rva = read_u32(bytes, opt + dir_off + 16)?;
    let res_size = read_u32(bytes, opt + dir_off + 20)?;
    if res_rva == 0 || res_size == 0 {
        return None;
    }
    let sections = read_sections(bytes, opt + size_opt, num_sections)?;
    let root = rva_to_off(&sections, res_rva)?;
    let version_entry = find_id_offset(bytes, root, 16)?;
    let data_entry = follow_to_data(bytes, root, version_entry)?;
    let data_rva = read_u32(bytes, data_entry)?;
    let data_off = rva_to_off(&sections, data_rva)?;
    parse_fixed_file_version(bytes.get(data_off..)?)
}

fn read_sections(bytes: &[u8], off: usize, count: usize) -> Option<Vec<Section>> {
    let mut sections = Vec::with_capacity(count);
    for i in 0..count {
        let s = off + i * 40;
        if bytes.len() < s + 40 {
            return None;
        }
        sections.push(Section {
            virtual_size: read_u32(bytes, s + 8)?,
            virtual_address: read_u32(bytes, s + 12)?,
            raw_size: read_u32(bytes, s + 16)?,
            raw_ptr: read_u32(bytes, s + 20)?,
        });
    }
    Some(sections)
}

fn rva_to_off(sections: &[Section], rva: u32) -> Option<usize> {
    for section in sections {
        let span = section.virtual_size.max(section.raw_size);
        if rva >= section.virtual_address && rva < section.virtual_address.saturating_add(span) {
            let delta = rva - section.virtual_address;
            if delta >= section.raw_size {
                return None;
            }
            return Some((section.raw_ptr + delta) as usize);
        }
    }
    None
}

fn find_id_offset(bytes: &[u8], dir_off: usize, id: u32) -> Option<u32> {
    let (named, ids, entries) = dir_entries(bytes, dir_off)?;
    let start = entries + named as usize * 8;
    for i in 0..ids as usize {
        let entry = start + i * 8;
        let name = read_u32(bytes, entry)?;
        if name & 0x8000_0000 == 0 && name == id {
            return read_u32(bytes, entry + 4);
        }
    }
    None
}

fn follow_to_data(bytes: &[u8], root: usize, mut offset_field: u32) -> Option<usize> {
    for _ in 0..4 {
        if offset_field & 0x8000_0000 != 0 {
            let dir = root + (offset_field & 0x7FFF_FFFF) as usize;
            let (named, ids, entries) = dir_entries(bytes, dir)?;
            let total = named as usize + ids as usize;
            if total == 0 {
                return None;
            }
            offset_field = read_u32(bytes, entries + 4)?;
        } else {
            return Some(root + offset_field as usize);
        }
    }
    None
}

fn dir_entries(bytes: &[u8], dir_off: usize) -> Option<(u16, u16, usize)> {
    let named = read_u16(bytes, dir_off + 12)?;
    let ids = read_u16(bytes, dir_off + 14)?;
    Some((named, ids, dir_off + 16))
}

fn parse_fixed_file_version(data: &[u8]) -> Option<String> {
    if data.len() < 6 {
        return None;
    }
    let value_len = read_u16(data, 2)? as usize;
    if value_len == 0 {
        return None;
    }
    let mut off = 6;
    loop {
        let ch = read_u16(data, off)?;
        off += 2;
        if ch == 0 {
            break;
        }
    }
    off = (off + 3) & !3;
    if read_u32(data, off)? != 0xFEEF04BD {
        return None;
    }
    let ms = read_u32(data, off + 8)?;
    let ls = read_u32(data, off + 12)?;
    let parts = [
        (ms >> 16) as u16,
        (ms & 0xFFFF) as u16,
        (ls >> 16) as u16,
        (ls & 0xFFFF) as u16,
    ];
    if parts == [0, 0, 0, 0] {
        return None;
    }
    Some(format!(
        "{}.{}.{}.{}",
        parts[0], parts[1], parts[2], parts[3]
    ))
}

fn read_u16(bytes: &[u8], off: usize) -> Option<u16> {
    let b = bytes.get(off..off + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32(bytes: &[u8], off: usize) -> Option<u32> {
    let b = bytes.get(off..off + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn push_u16(buf: &mut Vec<u8>, value: u16) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(buf: &mut Vec<u8>, value: u32) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    /// PE32+ with one resource section and a VS_FIXEDFILEINFO FileVersion.
    fn sample_pe(file_ms: u32, file_ls: u32) -> Vec<u8> {
        let mut version = Vec::new();
        let key: Vec<u8> = "VS_VERSION_INFO"
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(|c| c.to_le_bytes())
            .collect();
        let header = 6 + key.len();
        let aligned = (header + 3) & !3;
        let value_len = 52u16;
        let total = aligned + value_len as usize;
        push_u16(&mut version, total as u16);
        push_u16(&mut version, value_len);
        push_u16(&mut version, 0);
        version.extend_from_slice(&key);
        while version.len() < aligned {
            version.push(0);
        }
        push_u32(&mut version, 0xFEEF04BD);
        push_u32(&mut version, 0x0001_0000);
        push_u32(&mut version, file_ms);
        push_u32(&mut version, file_ls);
        while version.len() < total {
            version.push(0);
        }

        // Resource tree relative to the directory root:
        // 0x00 type dir, 0x10 type entry -> 0x18
        // 0x18 name dir, 0x28 name entry -> 0x30
        // 0x30 lang dir, 0x40 lang entry -> 0x48
        // 0x48 data entry, 0x58 version info
        let mut res = vec![0u8; 0x58];
        push_at(&mut res, 14, &1u16.to_le_bytes());
        push_at(&mut res, 0x10, &16u32.to_le_bytes());
        push_at(&mut res, 0x14, &(0x18u32 | 0x8000_0000).to_le_bytes());
        push_at(&mut res, 0x18 + 14, &1u16.to_le_bytes());
        push_at(&mut res, 0x28, &1u32.to_le_bytes());
        push_at(&mut res, 0x2C, &(0x30u32 | 0x8000_0000).to_le_bytes());
        push_at(&mut res, 0x30 + 14, &1u16.to_le_bytes());
        push_at(&mut res, 0x40, &0u32.to_le_bytes());
        push_at(&mut res, 0x44, &0x48u32.to_le_bytes());
        let version_rel = res.len() as u32;
        res.extend_from_slice(&version);
        let res_rva = 0x1000u32;
        push_at(&mut res, 0x48, &(res_rva + version_rel).to_le_bytes());
        push_at(&mut res, 0x4C, &(version.len() as u32).to_le_bytes());

        let e_lfanew = 0x40usize;
        let opt_size = 240usize; // PE32+ header through 16 data directories
        let section_off = e_lfanew + 4 + 20 + opt_size;
        let raw = (section_off + 40 + 0x1FF) & !0x1FF;
        let mut pe = vec![0u8; raw];
        pe[0] = b'M';
        pe[1] = b'Z';
        push_at(&mut pe, 0x3C, &(e_lfanew as u32).to_le_bytes());
        pe[e_lfanew..e_lfanew + 4].copy_from_slice(b"PE\0\0");
        let coff = e_lfanew + 4;
        push_at(&mut pe, coff + 2, &1u16.to_le_bytes());
        push_at(&mut pe, coff + 16, &(opt_size as u16).to_le_bytes());
        let opt = coff + 20;
        push_at(&mut pe, opt, &0x20Bu16.to_le_bytes());
        push_at(&mut pe, opt + 108, &16u32.to_le_bytes());
        // resource data directory is index 2
        push_at(&mut pe, opt + 112 + 16, &res_rva.to_le_bytes());
        push_at(&mut pe, opt + 112 + 20, &(res.len() as u32).to_le_bytes());
        // section header
        pe[section_off..section_off + 5].copy_from_slice(b".rsrc");
        push_at(&mut pe, section_off + 8, &(res.len() as u32).to_le_bytes());
        push_at(&mut pe, section_off + 12, &res_rva.to_le_bytes());
        push_at(&mut pe, section_off + 16, &(res.len() as u32).to_le_bytes());
        push_at(&mut pe, section_off + 20, &(raw as u32).to_le_bytes());
        pe.extend_from_slice(&res);
        pe
    }

    fn push_at(buf: &mut [u8], off: usize, bytes: &[u8]) {
        buf[off..off + bytes.len()].copy_from_slice(bytes);
    }

    #[test]
    fn reads_file_version_from_a_pe_resource() {
        let pe = sample_pe(0x0001_0006, 0x0492_0000);
        assert_eq!(file_version(&pe).as_deref(), Some("1.6.1170.0"));
    }

    #[test]
    fn ignores_a_zero_file_version() {
        let pe = sample_pe(0, 0);
        assert_eq!(file_version(&pe), None);
    }

    #[test]
    fn prefers_the_game_exe_over_setup() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("Skyrim");
        fs::create_dir_all(game.join("redist")).unwrap();
        let mut setup = fs::File::create(game.join("setup.exe")).unwrap();
        setup.write_all(b"MZ").unwrap();
        let mut shipping = fs::File::create(game.join("SkyrimSE.exe")).unwrap();
        shipping
            .write_all(&sample_pe(0x0001_0006, 0x0492_0000))
            .unwrap();
        assert_eq!(detect_install_version(&game).as_deref(), Some("1.6.1170.0"));
    }
}
