//! A path in JSON. Unix paths are bytes, and a directory whose name isn't
//! UTF-8 is still somewhere a shell can be, so such a path is written as its
//! bytes rather than failing the save or being restored somewhere else.

use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Repr {
    Text(String),
    Bytes { bytes: Vec<u8> },
}

pub fn serialize<S: Serializer>(path: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
    path.as_ref().map(|p| to_repr(p.clone())).serialize(s)
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
    Ok(Option::<Repr>::deserialize(d)?.map(from_repr))
}

fn to_repr(path: PathBuf) -> Repr {
    match path.into_os_string().into_string() {
        Ok(text) => Repr::Text(text),
        Err(raw) => Repr::Bytes {
            bytes: raw.into_encoded_bytes(),
        },
    }
}

fn from_repr(repr: Repr) -> PathBuf {
    match repr {
        Repr::Text(text) => PathBuf::from(text),
        Repr::Bytes { bytes } => bytes_to_path(bytes),
    }
}

#[cfg(unix)]
fn bytes_to_path(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn bytes_to_path(bytes: Vec<u8>) -> PathBuf {
    // Only Unix writes bytes, and elsewhere they mean nothing exact.
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}
