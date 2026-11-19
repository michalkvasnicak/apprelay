use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    time::Duration,
};

use cms::{content_info::ContentInfo, signed_data::SignedData};
use der::{Decode, asn1::OctetString};
use idevice::{
    afc::{AfcClient, opcode::AfcFopenMode},
    installation_proxy::InstallationProxyClient,
    lockdown::LockdownClient,
    remote_pairing::connect_tls_psk_tunnel_native,
    rsd::RsdHandshake,
    tcp::adapter::Adapter,
};
use tokio::{io::AsyncReadExt, time::timeout};
use uuid::Uuid;

use crate::{
    db::pairings::InstallTarget,
    error::OperationError,
    pairing::{connect_verified_pairing, dial_device},
};

const MAX_UNPACKED: u64 = 256 * 1024 * 1024;
const MAX_PROFILE: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;

pub(crate) struct Application {
    directory: tempfile::TempDir,
    entries: Vec<(PathBuf, bool)>,
    devices: Vec<String>,
}

impl Application {
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

pub(crate) async fn deliver(
    id: Uuid,
    mut target: InstallTarget,
    app: Application,
) -> Result<(), OperationError> {
    let mut control = connect_verified_pairing(
        target.address,
        target.port,
        &mut target.record,
        &target.peer_key,
    )
    .await?;
    let port = control
        .create_tcp_listener()
        .await
        .map_err(OperationError::at("tunnel-listener"))?;
    let stream = dial_device(target.address, port).await?;
    let tunnel = connect_tls_psk_tunnel_native(stream, control.encryption_key())
        .await
        .map_err(OperationError::at("tunnel-handshake"))?;
    let client_ip = tunnel
        .info
        .client_address
        .parse()
        .map_err(OperationError::at("tunnel-client-address"))?;
    let server_ip = tunnel
        .info
        .server_address
        .parse()
        .map_err(OperationError::at("tunnel-server-address"))?;
    let port = tunnel.info.server_rsd_port;
    let mtu = usize::from(tunnel.info.mtu);
    if port == 0 || !(1280..=16_000).contains(&mtu) {
        return Err(OperationError::without_source("tunnel-parameters"));
    }
    let mut adapter = Adapter::new(Box::new(tunnel.into_inner()), client_ip, server_ip);
    adapter.set_mss(mtu - 60);
    let mut handle = adapter.to_async_handle();
    let rsd_stream = handle
        .connect(port)
        .await
        .map_err(OperationError::at("rsd-connect"))?;
    let mut rsd = RsdHandshake::new(rsd_stream)
        .await
        .map_err(OperationError::at("rsd-handshake"))?;
    let mut lockdown = rsd
        .connect::<LockdownClient>(&mut handle)
        .await
        .map_err(OperationError::at("trusted-lockdown"))?;
    let udid = lockdown
        .get_value(Some("UniqueDeviceID"), None)
        .await
        .map_err(OperationError::at("device-udid"))?;
    if !udid.as_string().is_some_and(|udid| {
        !udid.is_empty()
            && app
                .devices
                .iter()
                .any(|entry| entry.eq_ignore_ascii_case(udid))
    }) {
        return Err(OperationError::without_source(
            "profile-does-not-include-device-udid",
        ));
    }
    drop(lockdown);

    let mut installer = rsd
        .connect::<InstallationProxyClient>(&mut handle)
        .await
        .map_err(OperationError::at("installation-service"))?;
    let mut afc = rsd
        .connect::<AfcClient>(&mut handle)
        .await
        .map_err(OperationError::at("afc-service"))?;
    if afc.get_file_info("PublicStaging").await.is_err() {
        afc.mk_dir("PublicStaging")
            .await
            .map_err(OperationError::at("staging-directory"))?;
    }
    let root = format!("PublicStaging/apprelay-{id}.app");
    let remaining = (target.expires_at - chrono::Utc::now())
        .to_std()
        .map_err(OperationError::at("install-expired"))?;
    let result = timeout(remaining.min(Duration::from_secs(480)), async {
        send_app(&mut afc, &app, &root).await?;
        let mut options = plist::Dictionary::new();
        options.insert("PackageType".into(), "Developer".into());
        installer
            .install(&root, Some(plist::Value::Dictionary(options)))
            .await
            .map_err(OperationError::at("device-install-failed"))
    })
    .await
    .map_err(OperationError::at("device-install-timeout"))
    .flatten();
    drop(afc);
    drop(installer);
    // Reopen AFC after interrupted framing. Never remove another job's staging.
    let _ = timeout(Duration::from_secs(5), async {
        let mut afc = rsd.connect::<AfcClient>(&mut handle).await?;
        afc.remove_all(&root).await
    })
    .await;
    result
}

async fn send_app(
    afc: &mut AfcClient,
    app: &Application,
    root: &str,
) -> Result<(), OperationError> {
    afc.mk_dir(root)
        .await
        .map_err(OperationError::at("staging-directory"))?;
    let mut bytes = vec![0; 1024 * 1024];
    for (path, directory) in &app.entries {
        let name = path
            .to_str()
            .ok_or_else(|| OperationError::without_source("invalid-ipa-path"))?;
        let remote = format!("{root}/{name}");
        if *directory {
            afc.mk_dir(remote)
                .await
                .map_err(OperationError::at("staging-directory"))?;
        } else {
            let mut source = tokio::fs::File::open(app.directory.path().join(path))
                .await
                .map_err(OperationError::at("staging-read"))?;
            let mut file = afc
                .open(remote, AfcFopenMode::WrOnly)
                .await
                .map_err(OperationError::at("staging-open"))?;
            loop {
                let len = source
                    .read(&mut bytes)
                    .await
                    .map_err(OperationError::at("staging-read"))?;
                if len == 0 {
                    break;
                }
                file.write_entire(&bytes[..len])
                    .await
                    .map_err(OperationError::at("staging-write"))?;
            }
            file.close()
                .await
                .map_err(OperationError::at("staging-close"))?;
        }
    }
    Ok(())
}
