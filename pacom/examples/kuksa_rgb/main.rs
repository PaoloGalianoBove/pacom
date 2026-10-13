use kuksa_rust_sdk::kuksa::common::ClientTraitV2;
use kuksa_rust_sdk::kuksa::val::v2::KuksaClientV2;
use kuksa_rust_sdk::v2_proto::Value;
use kuksa_rust_sdk::v2_proto::value::TypedValue;
use std::env;
use std::error::Error;
use std::io::{Error as IoError, ErrorKind};

const DEFAULT_SIGNAL: &str = "Vehicle.Cabin.Light.AmbientLight.Row1.DriverSide.Color";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let supplied_color = arguments.next().ok_or_else(|| {
        IoError::new(
            ErrorKind::InvalidInput,
            "usage: pacom-kuksa-rgb <#RRGGBB> [VSS_SIGNAL_PATH]",
        )
    })?;
    let signal = arguments
        .next()
        .unwrap_or_else(|| DEFAULT_SIGNAL.to_string());

    if arguments.next().is_some() {
        return Err(IoError::new(
            ErrorKind::InvalidInput,
            "usage: pacom-kuksa-rgb <#RRGGBB> [VSS_SIGNAL_PATH]",
        )
        .into());
    }

    let color = normalize_rgb_color(&supplied_color)?;
    let kuksa_uri =
        env::var("PACOM_KUKSA_URI").unwrap_or_else(|_| "http://127.0.0.1:55555".to_string());
    let uri: http::Uri = kuksa_uri.parse()?;
    let mut client = KuksaClientV2::new(uri);

    println!("[KUKSA RGB] Sending {color} to {signal} via {kuksa_uri}");
    client
        .actuate(
            signal.clone(),
            Value {
                typed_value: Some(TypedValue::String(color.clone())),
            },
        )
        .await?;
    println!("[KUKSA RGB] Actuation request sent successfully");

    Ok(())
}

fn normalize_rgb_color(input: &str) -> Result<String, IoError> {
    let hex = input.strip_prefix('#').unwrap_or(input);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(IoError::new(
            ErrorKind::InvalidInput,
            "RGB color must contain exactly six hexadecimal digits, for example #12A4FF",
        ));
    }

    Ok(format!("#{}", hex.to_ascii_uppercase()))
}
