use crate::service::ble::{BGC_CHAR_UUID, start_ble_client};

/// Read-only BLE diagnostic. With no address, scans and lists discovered devices
/// so you can find a Govee device's MAC. With an address, connects and reads the
/// BgcInfo characteristic to report reachability and the encryption version. It
/// never writes to the device or changes its state.
#[derive(clap::Parser, Debug)]
pub struct BleProbeCommand {
    /// BLE MAC to probe (e.g. A4:C1:38:11:22:33). If omitted, lists discovered
    /// devices instead.
    ble_address: Option<String>,
}

impl BleProbeCommand {
    pub async fn run(&self, _args: &crate::Args) -> anyhow::Result<()> {
        let Some(client) = start_ble_client().await? else {
            anyhow::bail!("No Bluetooth adapter found");
        };

        let Some(addr) = &self.ble_address else {
            println!("Scanning for BLE devices...");
            for (mac, name) in client.scan_list().await? {
                println!("  {mac}  {}", name.as_deref().unwrap_or("(no name)"));
            }
            return Ok(());
        };

        let report = client.probe(addr).await?;
        println!("address: {}", report.address);
        println!("advertisement manufacturer data (company id -> bytes):");
        for (company, bytes) in &report.manufacturer_data {
            println!("  {company:#06x}  {bytes:02x?}");
        }
        println!("advertisement service data (uuid -> bytes):");
        for (uuid, bytes) in &report.service_data {
            println!("  {uuid}  {bytes:02x?}");
        }
        println!("GATT characteristics (service / characteristic / properties):");
        for (service, uuid, props) in &report.characteristics {
            println!("  {service} / {uuid}  {props:?}");
        }
        match &report.bgc_info {
            Some(bgc) => {
                println!("BgcInfo bytes: {bgc:02x?}");
                match report.version {
                    Some(v) => println!("negotiated encryption version: {v:?}"),
                    None => println!("encryption version: UNKNOWN (byte[0] = {:?})", bgc.first()),
                }
            }
            None => println!("BgcInfo characteristic ({BGC_CHAR_UUID}) not found on this device"),
        }
        Ok(())
    }
}
