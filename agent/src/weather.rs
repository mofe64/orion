//! Location preferences and bounded, fixed-endpoint Open-Meteo requests.
use crate::{AgentConfig, memory};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Write, path::PathBuf, time::Duration};
pub const TIMEOUT: Duration = Duration::from_secs(5);
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Celsius,
    Fahrenheit,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub location: Option<Location>,
    pub temperature_unit: Option<Unit>,
}
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum When {
    #[default]
    Now,
    Today,
    Tomorrow,
    Week,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    #[serde(default)]
    pub when: When,
}
fn path(config: &AgentConfig) -> Result<PathBuf, String> {
    Ok(config
        .memory_path
        .as_deref()
        .ok_or("Weather storage is disabled")?
        .with_file_name("weather.json"))
}
impl Settings {
    fn validate(&self) -> Result<(), String> {
        if let Some(l) = &self.location
            && (l.name.trim().is_empty()
                || l.name.chars().count() > 200
                || l.name.chars().any(char::is_control)
                || !l.latitude.is_finite()
                || !(-90. ..=90.).contains(&l.latitude)
                || !l.longitude.is_finite()
                || !(-180. ..=180.).contains(&l.longitude)
                || l.timezone.parse::<chrono_tz::Tz>().is_err())
        {
            return Err("Invalid weather location".into());
        }
        Ok(())
    }
}
pub fn load(config: &AgentConfig) -> Result<Settings, String> {
    if config.memory_path.is_none() {
        return Ok(Settings::default());
    }
    let path = path(config)?;
    match std::fs::read(path) {
        Ok(bytes) => {
            if bytes.len() > 16384 {
                return Err("Weather settings are too large".into());
            }
            let settings: Settings = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            settings.validate()?;
            Ok(settings)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.to_string()),
    }
}
pub fn save(config: &AgentConfig, expected: Settings, settings: Settings) -> Result<(), String> {
    settings.validate()?;
    let path = path(config)?;
    let _lock = memory::edit_lock(&path)?;
    if load(config)? != expected {
        return Err("Weather settings changed. Refresh before saving.".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(path.parent().ok_or("Invalid weather path")?)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(&settings).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.without_url().to_string())
}
async fn read(mut response: reqwest::Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Weather provider returned HTTP {}",
            response.status()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("Weather response is too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid weather response".into())
}
pub async fn geocode(query: &str) -> Result<Vec<Location>, String> {
    if query.trim().is_empty() || query.chars().count() > 200 || query.chars().any(char::is_control)
    {
        return Err("Enter a location of 1–200 printable characters".into());
    }
    let value = read(
        client()?
            .get("https://geocoding-api.open-meteo.com/v1/search")
            .query(&[
                ("name", query.trim()),
                ("count", "10"),
                ("language", "en"),
                ("format", "json"),
            ])
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?,
    )
    .await?;
    let mut locations = vec![];
    for item in value["results"].as_array().into_iter().flatten() {
        let (Some(name), Some(lat), Some(lon), Some(zone)) = (
            item["name"].as_str(),
            item["latitude"].as_f64(),
            item["longitude"].as_f64(),
            item["timezone"].as_str(),
        ) else {
            continue;
        };
        let name = [
            Some(name),
            item["admin1"].as_str(),
            item["country"].as_str(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");
        let l = Location {
            name,
            latitude: lat,
            longitude: lon,
            timezone: zone.into(),
        };
        if (Settings {
            location: Some(l.clone()),
            temperature_unit: None,
        })
        .validate()
        .is_ok()
        {
            locations.push(l);
        }
    }
    Ok(locations)
}
fn unit(config: &AgentConfig, settings: &Settings) -> Result<(Unit, &'static str), String> {
    if let Some(unit) = settings.temperature_unit {
        return Ok((unit, "settings"));
    }
    if let Some(path) = config.memory_path.as_deref() {
        // Only explicit temperature preferences qualify; arbitrary mentions do not.
        let mut entries = memory::read(path)?;
        entries.sort_by(|a, b| b.created.cmp(&a.created));
        for entry in entries {
            let text = entry.text.to_lowercase();
            if text.contains("prefer") || text.contains("use") {
                if text.contains("fahrenheit") && !text.contains("celsius") {
                    return Ok((Unit::Fahrenheit, "memory"));
                }
                if text.contains("celsius") && !text.contains("fahrenheit") {
                    return Ok((Unit::Celsius, "memory"));
                }
            }
        }
    }
    Ok((Unit::Celsius, "default"))
}
pub async fn forecast(config: &AgentConfig, when: When) -> Result<Value, String> {
    let settings = load(config)?;
    let location = settings
        .location
        .as_ref()
        .ok_or("Set your location in Studio Settings before asking for weather")?;
    let (unit, source) = unit(config, &settings)?;
    let value=read(client()?.get("https://api.open-meteo.com/v1/forecast").query(&[("latitude",location.latitude.to_string()),("longitude",location.longitude.to_string()),("timezone",location.timezone.clone()),("temperature_unit",match unit { Unit::Celsius=>"celsius",Unit::Fahrenheit=>"fahrenheit" }.into()),("wind_speed_unit","kmh".into()),("forecast_days","7".into()),("current","temperature_2m,apparent_temperature,precipitation_probability,wind_speed_10m,weather_code".into()),("daily","weather_code,temperature_2m_max,temperature_2m_min,apparent_temperature_max,apparent_temperature_min,precipitation_probability_max,wind_speed_10m_max,sunrise,sunset".into())]).send().await.map_err(|e|e.without_url().to_string())?).await?;
    summary(value, location, unit, source, when)
}
fn summary(
    value: Value,
    location: &Location,
    unit: Unit,
    source: &str,
    when: When,
) -> Result<Value, String> {
    let days = value["daily"]["time"]
        .as_array()
        .ok_or("Weather provider omitted daily data")?;
    let range = match when {
        When::Tomorrow => 1..2,
        When::Week => 0..7,
        _ => 0..1,
    };
    if days.len() < range.end {
        return Err("Weather provider returned an incomplete forecast".into());
    }
    let daily:Vec<_>=range.map(|i| { let d=&value["daily"]; json!({"date":days[i],"high":d["temperature_2m_max"][i],"low":d["temperature_2m_min"][i],"feels_like_high":d["apparent_temperature_max"][i],"feels_like_low":d["apparent_temperature_min"][i],"precipitation_probability":d["precipitation_probability_max"][i],"wind_kmh":d["wind_speed_10m_max"][i],"sunrise":d["sunrise"][i],"sunset":d["sunset"][i],"description":description(d["weather_code"][i].as_u64())}) }).collect();
    let c = &value["current"];
    Ok(
        json!({"location":location.name,"timezone":location.timezone,"temperature_unit":unit,"unit_source":source,"current":if matches!(when,When::Now) { json!({"time":c["time"],"temperature":c["temperature_2m"],"feels_like":c["apparent_temperature"],"precipitation_probability":c["precipitation_probability"],"wind_kmh":c["wind_speed_10m"],"description":description(c["weather_code"].as_u64())}) } else {Value::Null},"daily":daily}),
    )
}
fn description(code: Option<u64>) -> &'static str {
    match code {
        Some(0) => "Clear sky",
        Some(1) => "Mainly clear",
        Some(2) => "Partly cloudy",
        Some(3) => "Overcast",
        Some(45 | 48) => "Fog",
        Some(51 | 53 | 55) => "Drizzle",
        Some(56 | 57) => "Freezing drizzle",
        Some(61 | 63 | 65) => "Rain",
        Some(66 | 67) => "Freezing rain",
        Some(71 | 73 | 75 | 77) => "Snow",
        Some(80..=82) => "Rain showers",
        Some(85 | 86) => "Snow showers",
        Some(95) => "Thunderstorm",
        Some(96 | 99) => "Thunderstorm with hail",
        _ => "Unknown weather code",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn settings_conflicts_units_and_missing_location() {
        let dir = tempfile::tempdir().unwrap();
        let config = AgentConfig {
            memory_path: Some(dir.path().join("MEMORY.md")),
            ..Default::default()
        };
        assert!(
            forecast(&config, When::Now)
                .await
                .unwrap_err()
                .contains("Studio Settings")
        );
        memory::append(
            config.memory_path.as_deref().unwrap(),
            "I prefer Fahrenheit for weather temperatures",
        )
        .unwrap();
        assert_eq!(
            unit(&config, &Settings::default()).unwrap(),
            (Unit::Fahrenheit, "memory")
        );
        let settings = Settings {
            location: Some(Location {
                name: "London".into(),
                latitude: 51.5,
                longitude: -0.1,
                timezone: "Europe/London".into(),
            }),
            temperature_unit: Some(Unit::Celsius),
        };
        save(&config, Settings::default(), settings.clone()).unwrap();
        assert_eq!(load(&config).unwrap(), settings);
        assert!(save(&config, Settings::default(), Settings::default()).is_err());
        assert_eq!(
            unit(&config, &settings).unwrap(),
            (Unit::Celsius, "settings")
        );
        assert!(serde_json::from_value::<Args>(json!({"when":"yesterday"})).is_err());
        assert!(serde_json::from_value::<Args>(json!({"url":"elsewhere"})).is_err());
        assert_eq!(TIMEOUT, Duration::from_secs(5));
    }
    #[test]
    fn compact_forecast_selects_day_and_describes_codes() {
        let l = Location {
            name: "London".into(),
            latitude: 51.5,
            longitude: 0.,
            timezone: "Europe/London".into(),
        };
        let value = json!({"daily":{"time":["today","tomorrow"],"weather_code":[0,61],"temperature_2m_max":[20,21],"temperature_2m_min":[10,11],"sunrise":["06:00","06:01"],"sunset":["18:00","17:59"]}});
        let result = summary(value, &l, Unit::Celsius, "default", When::Tomorrow).unwrap();
        assert_eq!(result["daily"][0]["description"], "Rain");
        assert_eq!(result["daily"][0]["high"], 21);
        assert_eq!(result["daily"].as_array().unwrap().len(), 1);
    }
}
