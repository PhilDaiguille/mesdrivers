//! Inventaire matériel via WMI.
use crate::model::{BrokenDevice, Device, wmi_date};
use serde::Deserialize;
use wmi::WMIConnection;

#[derive(Deserialize)]
#[serde(rename = "Win32_PnPSignedDriver", rename_all = "PascalCase")]
struct SignedDriver {
    device_name: Option<String>,
    driver_provider_name: Option<String>,
    driver_version: Option<String>,
    driver_date: Option<String>,
    #[serde(rename = "HardWareID")]
    hardware_id: Option<String>,
    inf_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename = "Win32_PnPEntity", rename_all = "PascalCase")]
struct PnpEntity {
    name: Option<String>,
    device_id: Option<String>,
    config_manager_error_code: Option<u32>,
}

pub fn scan(con: &WMIConnection) -> Result<(Vec<Device>, Vec<BrokenDevice>), wmi::WMIError> {
    let mut devices: Vec<Device> = con
        .query::<SignedDriver>()?
        .into_iter()
        .filter_map(|d| {
            Some(Device {
                name: d.device_name?,
                provider: d.driver_provider_name.unwrap_or_default(),
                version: d.driver_version.unwrap_or_default(),
                date: wmi_date(d.driver_date.as_deref().unwrap_or_default()),
                hwid: d.hardware_id.unwrap_or_default(),
                inf: d.inf_name.unwrap_or_default(),
                ..Default::default()
            })
        })
        .collect();
    devices.sort_by_key(|d| d.name.to_lowercase());

    let broken = con
        .raw_query::<PnpEntity>("SELECT Name, DeviceID, ConfigManagerErrorCode FROM Win32_PnPEntity WHERE ConfigManagerErrorCode <> 0")?
        .into_iter()
        .map(|e| BrokenDevice {
            name: e.name.or(e.device_id).unwrap_or_else(|| "Périphérique inconnu".into()),
            code: e.config_manager_error_code.unwrap_or(0),
        })
        .collect();

    Ok((devices, broken))
}
