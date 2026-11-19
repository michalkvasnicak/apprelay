use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
};

use cms::{content_info::ContentInfo, signed_data::SignedData};
use der::{Decode, asn1::OctetString};

use crate::error::OperationError;

const MAX_UNPACKED: u64 = 256 * 1024 * 1024;
const MAX_PROFILE: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;

pub(crate) struct UnpackedIpa {
    directory: tempfile::TempDir,
    entries: Vec<(PathBuf, bool)>,
    devices: Vec<String>,
}

impl UnpackedIpa {
    pub(crate) fn unpack(ipa: Vec<u8>) -> Result<Self, OperationError> {
        let directory = tempfile::tempdir().map_err(OperationError::at("local-staging"))?;
        let mut zip =
            zip::ZipArchive::new(Cursor::new(ipa)).map_err(OperationError::at("invalid-ipa"))?;
        if zip.len() > MAX_ENTRIES {
            return Err(OperationError::without_source("ipa-too-many-files"));
        }
        let mut root = None;
        let mut seen = HashSet::new();
        let mut total = 0;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).map_err(OperationError::at("invalid-ipa"))?;
            let name = std::str::from_utf8(file.name_raw())
                .map_err(OperationError::at("invalid-ipa-path"))?;
            let path = name.strip_suffix('/').unwrap_or(name);
            if path.len() > 1024
                || path.split('/').count() > 40
                || path.split('/').any(|part| {
                    part.is_empty()
                        || part == "."
                        || part == ".."
                        || part.contains('\\')
                        || part.chars().any(char::is_control)
                })
                || file.enclosed_name().is_none()
            {
                return Err(OperationError::without_source("invalid-ipa-path"));
            }
            let Some((app, relative)) = path
                .strip_prefix("Payload/")
                .map(|p| p.split_once('/').unwrap_or((p, "")))
            else {
                continue;
            };
            if !app.ends_with(".app") {
                continue;
            }
            if root.get_or_insert_with(|| app.to_owned()).as_str() != app {
                return Err(OperationError::without_source("ipa-has-multiple-apps"));
            }
            if !seen.insert(relative.to_owned()) {
                return Err(OperationError::without_source("duplicate-ipa-path"));
            }
            let is_dir = file.is_dir();
            let kind = file.unix_mode().unwrap_or(0) & 0o170000;
            if kind != 0 && kind != if is_dir { 0o040000 } else { 0o100000 } {
                return Err(OperationError::without_source("unsupported-ipa-entry"));
            }
            if relative.is_empty() && !is_dir {
                return Err(OperationError::without_source("invalid-app-root"));
            }
            let dest = directory.path().join(relative);
            if is_dir {
                std::fs::create_dir_all(dest)
                    .map_err(OperationError::at("local-staging-directory"))?;
            } else {
                let size = file.size();
                if size > MAX_UNPACKED - total {
                    return Err(OperationError::without_source("ipa-expanded-size-limit"));
                }
                total += size;
                let parent = dest
                    .parent()
                    .ok_or_else(|| OperationError::without_source("invalid-ipa-path"))?;
                std::fs::create_dir_all(parent)
                    .map_err(OperationError::at("local-staging-directory"))?;
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(dest)
                    .map_err(OperationError::at("local-staging-file"))?;
                if io::copy(&mut file.by_ref().take(size + 1), &mut output)
                    .map_err(OperationError::at("ipa-extraction"))?
                    != size
                {
                    return Err(OperationError::without_source("ipa-entry-size-mismatch"));
                }
            }
        }
        if root.is_none() || !directory.path().join("Info.plist").is_file() {
            return Err(OperationError::without_source("ipa-missing-app"));
        }
        let devices = provisioned_devices(&directory.path().join("embedded.mobileprovision"))?;
        let mut entries = Vec::new();
        collect_files(directory.path(), Path::new(""), &mut entries)?;
        Ok(Self {
            directory,
            entries,
            devices,
        })
    }

    pub(super) fn root(&self) -> &Path {
        self.directory.path()
    }

    pub(super) fn entries(&self) -> &[(PathBuf, bool)] {
        &self.entries
    }

    pub(super) fn is_provisioned_for(&self, udid: &str) -> bool {
        !udid.is_empty()
            && self
                .devices
                .iter()
                .any(|entry| entry.eq_ignore_ascii_case(udid))
    }
}

fn provisioned_devices(profile: &Path) -> Result<Vec<String>, OperationError> {
    let mut bytes = Vec::new();
    std::fs::File::open(profile)
        .map_err(OperationError::at("missing-provisioning-profile"))?
        .take(MAX_PROFILE + 1)
        .read_to_end(&mut bytes)
        .map_err(OperationError::at("invalid-provisioning-profile"))?;
    if bytes.len() as u64 > MAX_PROFILE {
        return Err(OperationError::without_source("profile-too-large"));
    }
    let contents = ContentInfo::from_der(&bytes)
        .map_err(OperationError::at("invalid-provisioning-profile"))?;
    let signed = contents
        .content
        .decode_as::<SignedData>()
        .map_err(OperationError::at("invalid-provisioning-profile"))?;
    let body = signed
        .encap_content_info
        .econtent
        .ok_or_else(|| OperationError::without_source("invalid-provisioning-profile"))?
        .decode_as::<OctetString>()
        .map_err(OperationError::at("invalid-provisioning-profile"))?;
    // This is a target-UDID preflight; iOS validates the actual signature.
    let profile = plist::Value::from_reader(Cursor::new(body.as_bytes()))
        .map_err(OperationError::at("invalid-provisioning-profile"))?;
    let devices = profile
        .as_dictionary()
        .and_then(|d| d.get("ProvisionedDevices"))
        .and_then(plist::Value::as_array)
        .ok_or_else(|| OperationError::without_source("profile-has-no-provisioned-devices"))?;
    Ok(devices
        .iter()
        .filter_map(plist::Value::as_string)
        .map(str::to_owned)
        .collect())
}

fn collect_files(
    root: &Path,
    relative: &Path,
    entries: &mut Vec<(PathBuf, bool)>,
) -> Result<(), OperationError> {
    let mut pending = vec![relative.to_owned()];
    while let Some(parent) = pending.pop() {
        let children =
            std::fs::read_dir(root.join(&parent)).map_err(OperationError::at("local-staging"))?;
        for child in children {
            let child = child.map_err(OperationError::at("local-staging"))?;
            let path = parent.join(child.file_name());
            let directory = child
                .file_type()
                .map_err(OperationError::at("local-staging"))?
                .is_dir();
            if entries.len() >= MAX_ENTRIES {
                return Err(OperationError::without_source("ipa-too-many-files"));
            }
            if directory {
                pending.push(path.clone());
            }
            entries.push((path, directory));
        }
    }
    Ok(())
}
