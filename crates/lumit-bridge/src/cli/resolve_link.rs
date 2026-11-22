//! One-shot Windows menu setup. Resolve itself is accessed only by its Lua script.
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};
use windows::{
    core::{w, HSTRING, PCSTR},
    Win32::{
        Foundation::{FreeLibrary, BOOL, HWND, LPARAM, WPARAM},
        System::LibraryLoader::{GetProcAddress, LoadLibraryW},
        UI::WindowsAndMessaging::{
            EnumWindows, GetPropW, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_COPYDATA,
        },
    },
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const PLUGIN: &[u8] = b"com.lumitlab.resolve.preview";
#[derive(Clone)]
struct Field {
    tag: u64,
    bytes: Vec<u8>,
}
fn varint(data: &[u8], at: &mut usize) -> Result<u64> {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let byte = *data.get(*at).ok_or("Truncated Resolve preset")?;
        *at += 1;
        value |= u64::from(byte & 127)
            .checked_shl(shift)
            .ok_or("Invalid preset field")?;
        if byte < 128 {
            return Ok(value);
        }
    }
    Err("Invalid preset field".into())
}
fn fields(data: &[u8]) -> Result<Vec<Field>> {
    let mut at = 0;
    let mut out = Vec::new();
    while at < data.len() {
        let tag = varint(data, &mut at)?;
        let bytes = match tag & 7 {
            0 => {
                let start = at;
                varint(data, &mut at)?;
                data[start..at].to_vec()
            }
            wire => {
                let size = match wire {
                    2 => usize::try_from(varint(data, &mut at)?)?,
                    1 => 8,
                    5 => 4,
                    _ => return Err("Unsupported preset field".into()),
                };
                let end = at.checked_add(size).ok_or("Invalid field size")?;
                let bytes = data.get(at..end).ok_or("Truncated preset field")?.to_vec();
                at = end;
                bytes
            }
        };
        out.push(Field { tag, bytes });
    }
    Ok(out)
}
fn number(mut n: u64, out: &mut Vec<u8>) {
    while n >= 128 {
        out.push(n as u8 | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn encode(fields: &[Field]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in fields {
        number(f.tag, &mut out);
        if f.tag & 7 == 2 {
            number(f.bytes.len() as u64, &mut out);
        }
        out.extend_from_slice(&f.bytes);
    }
    out
}
fn visit(
    data: &[u8],
    route: &[u64],
    values: &BTreeMap<String, String>,
    found: &mut usize,
) -> Result<Vec<u8>> {
    let mut fs = fields(data)?;
    if let Some((tag, rest)) = route.split_first() {
        for f in &mut fs {
            if f.tag == *tag {
                f.bytes = visit(&f.bytes, rest, values, found)?;
            }
        }
    } else if fs.iter().any(|f| f.tag == 18 && f.bytes == PLUGIN) {
        *found += 1;
        fs.retain(|f| {
            f.tag != 42
                || fields(&f.bytes).map_or(true, |p| {
                    !p.iter()
                        .any(|f| f.tag == 10 && values.keys().any(|n| n.as_bytes() == f.bytes))
                })
        });
        for (name, value) in values {
            fs.push(Field {
                tag: 42,
                bytes: encode(&[
                    Field {
                        tag: 10,
                        bytes: name.as_bytes().to_vec(),
                    },
                    Field {
                        tag: 18,
                        bytes: encode(&[Field {
                            tag: 42,
                            bytes: value.as_bytes().to_vec(),
                        }]),
                    },
                ]),
            });
        }
    }
    Ok(encode(&fs))
}
fn unpack(data: &[u8], codec: &Path) -> Result<Vec<u8>> {
    if data.len() < 9
        || data[..4] != 2u32.to_be_bytes()
        || data[4..8] != (data.len() as u32 - 8).to_be_bytes()
    {
        return Err("Unsupported Resolve preset envelope".into());
    }
    match data[8] {
        128 => Ok(data[9..].to_vec()),
        129 => unsafe {
            let library = LoadLibraryW(&HSTRING::from(codec.as_os_str()))?;
            let answer = (|| -> Result<Vec<u8>> {
                let size: unsafe extern "C" fn(*const u8, usize) -> u64 = std::mem::transmute(
                    GetProcAddress(library, PCSTR(c"ZSTD_getFrameContentSize".as_ptr().cast()))
                        .ok_or("Missing preset codec")?,
                );
                let decode: unsafe extern "C" fn(*mut u8, usize, *const u8, usize) -> usize =
                    std::mem::transmute(
                        GetProcAddress(library, PCSTR(c"ZSTD_decompress".as_ptr().cast()))
                            .ok_or("Missing preset codec")?,
                    );
                let size = size(data[9..].as_ptr(), data.len() - 9);
                if size == 0 || size > 4 << 20 {
                    return Err("Invalid preset size".into());
                }
                let mut out = vec![0; size as usize];
                if decode(
                    out.as_mut_ptr(),
                    out.len(),
                    data[9..].as_ptr(),
                    data.len() - 9,
                ) != out.len()
                {
                    return Err("Invalid compressed preset".into());
                }
                Ok(out)
            })();
            let _ = FreeLibrary(library);
            answer
        },
        _ => Err("Unsupported preset compression".into()),
    }
}
fn preset(
    source: &Path,
    destination: &Path,
    codec: &Path,
    values: &BTreeMap<String, String>,
) -> Result<()> {
    let mut archive = zip::ZipArchive::new(fs::File::open(source)?)?;
    let mut entries = Vec::new();
    let mut found = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name()?.into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if name.starts_with("SeqContainer/") {
            let mut xml = String::from_utf8(bytes)?;
            let mut at = 0;
            while let Some(start) = xml[at..].find("<EffectFiltersBA>").map(|n| n + at + 17) {
                let end = start
                    + xml[start..]
                        .find("</EffectFiltersBA>")
                        .ok_or("Incomplete preset")?;
                let data = xml.as_bytes()[start..end]
                    .chunks(2)
                    .map(|c| Ok(u8::from_str_radix(std::str::from_utf8(c)?, 16)?))
                    .collect::<Result<Vec<u8>>>()?;
                if data.len() >= 9 {
                    let raw = unpack(&data, codec)?;
                    if raw.windows(PLUGIN.len()).any(|p| p == PLUGIN) {
                        let patched = visit(&raw, &[10, 74, 26, 18], values, &mut found)?;
                        let mut data = Vec::new();
                        data.extend_from_slice(&2u32.to_be_bytes());
                        data.extend_from_slice(&(patched.len() as u32 + 1).to_be_bytes());
                        data.push(128);
                        data.extend_from_slice(&patched);
                        let hex = data.iter().map(|b| format!("{b:02x}")).collect::<String>();
                        xml.replace_range(start..end, &hex);
                        at = start + hex.len();
                        continue;
                    }
                }
                at = end;
            }
            bytes = xml.into_bytes();
        }
        entries.push((name, bytes));
    }
    if found != 1 {
        return Err("The setup preset must contain one Lumit Composition effect".into());
    }
    let mut zip = zip::ZipWriter::new(fs::File::create(destination)?);
    for (name, bytes) in entries {
        zip.start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )?;
        zip.write_all(&bytes)?;
    }
    zip.finish()?;
    Ok(())
}
unsafe extern "system" fn find_window(window: HWND, data: LPARAM) -> BOOL {
    if !GetPropW(window, w!("LumitOpen")).is_invalid() {
        (&mut *(data.0 as *mut Vec<HWND>)).push(window);
    }
    BOOL(1)
}
fn windows() -> Result<Vec<HWND>> {
    let mut out = Vec::new();
    unsafe {
        EnumWindows(Some(find_window), LPARAM(&mut out as *mut _ as isize))?;
    }
    Ok(out)
}
#[repr(C)]
struct CopyData {
    magic: usize,
    size: u32,
    data: *const u8,
}
fn ask_editor(editor: &Path, request: &Path) -> Result<Value> {
    let open = windows()?;
    let argument = format!("--resolve-import={}", request.display());
    if open.is_empty() {
        Command::new(editor.join("lumit.exe"))
            .arg(&argument)
            .creation_flags(0x08000000)
            .spawn()?;
    } else if open.len() != 1 {
        return Err("Keep one Lumit window open, then try again".into());
    } else {
        let mut text = argument.into_bytes();
        text.push(0);
        let copy = CopyData {
            magic: 0x4c554d54,
            size: text.len() as u32,
            data: text.as_ptr(),
        };
        let mut result = 0;
        if unsafe {
            SendMessageTimeoutW(
                open[0],
                WM_COPYDATA,
                WPARAM(0),
                LPARAM(&copy as *const _ as isize),
                SMTO_ABORTIFHUNG,
                5000,
                Some(&mut result),
            )
        }
        .0 == 0
        {
            return Err("Lumit did not accept the request".into());
        }
    }
    let reply = request.with_file_name("imported.json");
    let until = Instant::now() + Duration::from_secs(300);
    while !reply.exists() && Instant::now() < until {
        thread::sleep(Duration::from_millis(100));
    }
    if !reply.exists() {
        return Err("Lumit did not finish the request in time".into());
    }
    let answer: Value = serde_json::from_slice(&fs::read(reply)?)?;
    if let Some(error) = answer["error"].as_str() {
        return Err(error.to_owned().into());
    }
    Ok(answer)
}
// The codec is loaded into this signed program, so it has to come from
// Resolve's install folder or Lumit's own.
fn trusted_codec(codec: &Path, editor: &Path) -> Result<PathBuf> {
    let codec = fs::canonicalize(codec)?;
    let resolve = std::env::var_os("ProgramFiles")
        .map(|folder| PathBuf::from(folder).join("Blackmagic Design/DaVinci Resolve"));
    let trusted = [Some(editor.to_path_buf()), resolve]
        .into_iter()
        .flatten()
        .filter_map(|folder| fs::canonicalize(folder).ok())
        .any(|folder| codec.starts_with(folder));
    if !trusted {
        return Err("The preset codec is not in the Resolve or Lumit install folder".into());
    }
    Ok(codec)
}
fn prepare(directory: &Path) -> Result<Value> {
    let spec: Value = serde_json::from_slice(&fs::read(directory.join("selection.json"))?)?;
    let executable = std::env::current_exe()?;
    let editor = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("Missing Lumit app folder")?;
    let codec = trusted_codec(
        Path::new(spec["codec"].as_str().ok_or("Missing preset codec")?),
        editor,
    )?;
    let seed = directory.join("seed.drt");
    let source = directory.join("selection.lum");
    super::run(
        &editor.join("Resolve/import.js"),
        None,
        super::Save::As(source.clone()),
        BTreeMap::from([(
            "selection".into(),
            directory
                .join("selection.json")
                .to_string_lossy()
                .into_owned(),
        )]),
    )
    .map_err(std::io::Error::other)?;
    let request = json!({"project":spec["project"],"projectId":spec["projectId"],"project_name":spec["project_name"],"source":source});

    let path = directory.join("request.json");
    fs::write(&path, serde_json::to_vec(&request)?)?;
    let answer = ask_editor(editor, &path)?;
    if answer["cancelled"].is_string() {
        return Ok(answer);
    }
    let values = ["project", "projectId", "composition"]
        .into_iter()
        .map(|key| {
            Ok((
                key.to_owned(),
                answer[key].as_str().ok_or("Missing Lumit link")?.to_owned(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    preset(&seed, &directory.join("configured.drt"), &codec, &values)?;

    Ok(answer)
}
pub(super) fn run(directory: &Path) -> std::result::Result<(), String> {
    let answer = match prepare(directory) {
        Ok(value) => value,
        Err(error) => json!({"error":error.to_string()}),
    };
    let values = answer.as_object().ok_or("Invalid setup response")?;
    let mut lua = String::from("return {");
    for (key, value) in values {
        if let Some(value) = value.as_str() {
            let mut equals = String::from("=");
            while value.contains(&format!("]{equals}]")) {
                equals.push('=');
            }
            lua.push_str(&format!("{key}=[{equals}[{value}]{equals}],"));
        }
    }
    lua.push('}');
    let pending = directory.join("ready.tmp");
    fs::write(&pending, lua).map_err(|e| e.to_string())?;
    fs::rename(pending, directory.join("ready.lua")).map_err(|e| e.to_string())
}
