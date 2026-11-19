use std::time::Duration;

use chrono::{DateTime, Utc};
use idevice::{
    afc::{AfcClient, opcode::AfcFopenMode},
    installation_proxy::InstallationProxyClient,
    lockdown::LockdownClient,
};
use tokio::{io::AsyncReadExt, time::timeout};
use uuid::Uuid;

use super::session::IphoneSession;
use crate::error::OperationError;

mod ipa_bundle;

pub(crate) use ipa_bundle::UnpackedIpa;

pub(crate) async fn deliver(
    id: Uuid,
    phone: &mut IphoneSession,
    app: UnpackedIpa,
    expires_at: DateTime<Utc>,
) -> Result<(), OperationError> {
    let mut lockdown = phone
        .connect_service::<LockdownClient>()
        .await
        .map_err(OperationError::at("trusted-lockdown"))?;
    let udid = lockdown
        .get_value(Some("UniqueDeviceID"), None)
        .await
        .map_err(OperationError::at("device-udid"))?;
    if !udid
        .as_string()
        .is_some_and(|udid| app.is_provisioned_for(udid))
    {
        return Err(OperationError::without_source(
            "profile-does-not-include-device-udid",
        ));
    }
    drop(lockdown);

    let mut installer = phone
        .connect_service::<InstallationProxyClient>()
        .await
        .map_err(OperationError::at("installation-service"))?;
    let mut afc = phone
        .connect_service::<AfcClient>()
        .await
        .map_err(OperationError::at("afc-service"))?;
    if afc.get_file_info("PublicStaging").await.is_err() {
        afc.mk_dir("PublicStaging")
            .await
            .map_err(OperationError::at("staging-directory"))?;
    }
    let root = format!("PublicStaging/apprelay-{id}.app");
    let remaining = (expires_at - Utc::now())
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
        let mut afc = phone.connect_service::<AfcClient>().await?;
        afc.remove_all(&root).await
    })
    .await;
    result
}

async fn send_app(
    afc: &mut AfcClient,
    app: &UnpackedIpa,
    root: &str,
) -> Result<(), OperationError> {
    afc.mk_dir(root)
        .await
        .map_err(OperationError::at("staging-directory"))?;
    let mut bytes = vec![0; 1024 * 1024];
    for (path, directory) in app.entries() {
        let name = path
            .to_str()
            .ok_or_else(|| OperationError::without_source("invalid-ipa-path"))?;
        let remote = format!("{root}/{name}");
        if *directory {
            afc.mk_dir(remote)
                .await
                .map_err(OperationError::at("staging-directory"))?;
        } else {
            let mut source = tokio::fs::File::open(app.root().join(path))
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
