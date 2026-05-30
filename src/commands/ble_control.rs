use crate::service::ble::start_ble_client;
use govee_api::ble::{Base64HexBytes, SetDevicePower};

/// Send a single control frame to a device over direct BLE. Connects, negotiates
/// the session (plaintext or V1/V2 depending on the device), and writes one frame.
/// This DOES change device state. Used to confirm direct BLE control against real
/// hardware.
///
/// Either `--power on|off` (the generic SetDevicePower frame) or `--frame <hex>`
/// (a raw command, e.g. an SKU-specific frame from a capture). With `--frame`, the
/// bytes are padded to 19 and a trailing XOR checksum is appended, matching the
/// 20-byte Govee frame format.
#[derive(clap::Parser, Debug)]
pub struct BleControlCommand {
    /// BLE MAC of the device (e.g. C1:9A:45:A3:45:C3).
    ble_address: String,

    /// Generic power state.
    #[arg(long, value_parser = ["on", "off"], conflicts_with = "frame")]
    power: Option<String>,

    /// Raw frame as hex (e.g. "3311010a0f0f01031f0101ff07ce01"); padded to 19
    /// bytes with a trailing XOR checksum.
    #[arg(long)]
    frame: Option<String>,
}

impl BleControlCommand {
    pub async fn run(&self, _args: &crate::Args) -> anyhow::Result<()> {
        let bytes = match (&self.power, &self.frame) {
            (Some(p), None) => {
                let on = p == "on";
                // SetDevicePower is registered under the generic light codec.
                Base64HexBytes::encode_for_sku("Generic:Light", &SetDevicePower { on })?
                    .bytes()
                    .to_vec()
            }
            (None, Some(hex)) => finish_frame(parse_hex(hex)?),
            _ => anyhow::bail!("provide exactly one of --power or --frame"),
        };

        let Some(client) = start_ble_client(crate::resolve_timezone()).await? else {
            anyhow::bail!("No Bluetooth adapter found");
        };
        println!(
            "sending to {} ({} bytes): {:02x?}",
            self.ble_address,
            bytes.len(),
            bytes
        );
        // No SKU context here, so no family-specific session init runs; this is a
        // raw one-shot frame sender.
        let result = client
            .send_frames(&self.ble_address, "", &[bytes], None)
            .await;
        // release the device so it advertises again for the next attempt.
        client.disconnect(&self.ble_address).await;
        result?;
        println!("written");
        Ok(())
    }
}

fn parse_hex(s: &str) -> anyhow::Result<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    anyhow::ensure!(
        s.len().is_multiple_of(2),
        "hex must have an even number of digits"
    );
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into))
        .collect()
}

/// Pad to 19 bytes and append a trailing XOR checksum (the 20-byte Govee frame).
fn finish_frame(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.resize(19, 0);
    let checksum = bytes.iter().fold(0u8, |a, b| a ^ b);
    bytes.push(checksum);
    bytes
}
