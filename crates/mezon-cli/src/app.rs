use mezon_native::control::ControlClient;
use serde_json::Value;

pub fn logout() -> anyhow::Result<()> {
    ControlClient::request("app.logout", Value::Null)?;
    println!("Logged out");
    Ok(())
}

pub fn refresh() -> anyhow::Result<()> {
    ControlClient::request("app.refresh", Value::Null)?;
    println!("Refreshed");
    Ok(())
}

pub fn quit() -> anyhow::Result<()> {
    ControlClient::request("app.quit", Value::Null)?;
    println!("Quit requested");
    Ok(())
}

pub fn status() -> anyhow::Result<()> {
    let result = ControlClient::request("app.status", Value::Null)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
