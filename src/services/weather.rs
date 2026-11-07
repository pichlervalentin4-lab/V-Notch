//! Forecast from Open-Meteo. No account or key needed.

use super::http;
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HourForecast {
    pub time: String,
    pub temperature: f32,
    pub rain_chance: f32,
    pub rain: f32,
    pub code: i32,
    pub is_day: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DayForecast {
    pub date: String,
    pub code: i32,
    pub max: f32,
    pub min: f32,
    pub rain_chance: f32,
    pub uv: f32,
    pub sunrise: String,
    pub sunset: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Forecast {
    pub place: String,
    pub temperature: f32,
    pub feels_like: f32,
    pub humidity: f32,
    pub wind: f32,
    pub gusts: f32,
    /// Where the wind comes from, in degrees (0 = north), for the wind tile's dial.
    pub wind_direction: f32,
    pub code: i32,
    pub is_day: bool,
    pub hours: Vec<HourForecast>,
    pub days: Vec<DayForecast>,
    /// Local time of the download, "HH:MM", shown as "Updated …".
    pub fetched_at: String,
}

pub fn geocode(city: &str) -> Result<Place, String> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=de&format=json",
        http::encode(city)
    );
    let value = http::get_json(&url, TIMEOUT)?;
    let first = value.pointer("/results/0").ok_or("city not found")?;
    Ok(Place {
        name: first["name"].as_str().unwrap_or(city).to_string(),
        lat: first["latitude"].as_f64().ok_or("no latitude")?,
        lon: first["longitude"].as_f64().ok_or("no longitude")?,
    })
}

/// Rough location from the IP address, used when no city is set.
pub fn locate_by_ip() -> Result<Place, String> {
    let value = http::get_json("https://ipwho.is/", TIMEOUT)?;
    if value["success"].as_bool() == Some(false) {
        return Err("location lookup refused".into());
    }
    Ok(Place {
        name: value["city"].as_str().unwrap_or_default().to_string(),
        lat: value["latitude"].as_f64().ok_or("no latitude")?,
        lon: value["longitude"].as_f64().ok_or("no longitude")?,
    })
}

pub fn fetch(place: &Place, fahrenheit: bool) -> Result<Forecast, String> {
    let unit = if fahrenheit { "&temperature_unit=fahrenheit" } else { "" };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,is_day,weather_code,wind_speed_10m,wind_gusts_10m,wind_direction_10m\
         &hourly=temperature_2m,precipitation_probability,precipitation,weather_code,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,uv_index_max,sunrise,sunset\
         &timezone=auto&forecast_days=7{unit}",
        place.lat, place.lon
    );
    parse_forecast(&http::get_json(&url, TIMEOUT)?, &place.name)
}

fn number(value: &Value, index: usize) -> f32 {
    value.get(index).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

fn text(value: &Value, index: usize) -> String {
    value.get(index).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn parse_forecast(root: &Value, place: &str) -> Result<Forecast, String> {
    let current = root.get("current").ok_or("no current weather")?;
    let hourly = root.get("hourly").ok_or("no hourly forecast")?;
    let daily = root.get("daily").ok_or("no daily forecast")?;
    let read = |key: &str| current[key].as_f64().unwrap_or(0.0) as f32;

    let hour_count = hourly["time"].as_array().map_or(0, Vec::len);
    let current_time = current["time"].as_str().unwrap_or_default();
    // The first hour that has not passed yet, so the chart starts at "now".
    let start = hourly["time"]
        .as_array()
        .and_then(|times| {
            let hour = current_time.get(..13).unwrap_or_default();
            times.iter().position(|t| t.as_str().is_some_and(|s| s.starts_with(hour)))
        })
        .unwrap_or(0);
    let hours = (start..hour_count.min(start + 24))
        .map(|i| HourForecast {
            time: text(&hourly["time"], i),
            temperature: number(&hourly["temperature_2m"], i),
            rain_chance: number(&hourly["precipitation_probability"], i),
            rain: number(&hourly["precipitation"], i),
            code: number(&hourly["weather_code"], i) as i32,
            is_day: number(&hourly["is_day"], i) > 0.5,
        })
        .collect();

    let day_count = daily["time"].as_array().map_or(0, Vec::len);
    let days = (0..day_count)
        .map(|i| DayForecast {
            date: text(&daily["time"], i),
            code: number(&daily["weather_code"], i) as i32,
            max: number(&daily["temperature_2m_max"], i),
            min: number(&daily["temperature_2m_min"], i),
            rain_chance: number(&daily["precipitation_probability_max"], i),
            uv: number(&daily["uv_index_max"], i),
            sunrise: text(&daily["sunrise"], i),
            sunset: text(&daily["sunset"], i),
        })
        .collect();

    Ok(Forecast {
        place: place.to_string(),
        temperature: read("temperature_2m"),
        feels_like: read("apparent_temperature"),
        humidity: read("relative_humidity_2m"),
        wind: read("wind_speed_10m"),
        gusts: read("wind_gusts_10m"),
        wind_direction: read("wind_direction_10m"),
        code: current["weather_code"].as_i64().unwrap_or(0) as i32,
        is_day: current["is_day"].as_i64().unwrap_or(1) == 1,
        hours,
        days,
        fetched_at: chrono::Local::now().format("%H:%M").to_string(),
    })
}

/// The locale key of a WMO weather code.
pub fn condition_key(code: i32) -> &'static str {
    match code {
        0 => "weather.condition.clear",
        1 => "weather.condition.mainlyClear",
        2 => "weather.condition.partlyCloudy",
        3 => "weather.condition.cloudy",
        45 | 48 => "weather.condition.fog",
        51 | 53 | 55 => "weather.condition.drizzle",
        56 | 57 => "weather.condition.freezingDrizzle",
        61 | 63 | 65 => "weather.condition.rain",
        66 | 67 => "weather.condition.freezingRain",
        71 | 73 | 75 => "weather.condition.snow",
        77 => "weather.condition.snowGrains",
        80..=82 => "weather.condition.rainShowers",
        85 | 86 => "weather.condition.snowShowers",
        95 | 96 | 99 => "weather.condition.thunderstorm",
        _ => "weather.condition.unknown",
    }
}

pub fn condition_icon(code: i32, is_day: bool) -> &'static str {
    use crate::icons::*;
    match code {
        0 | 1 => if is_day { SUN } else { MOON_STARS },
        2 => if is_day { CLOUD_SUN } else { CLOUD_MOON },
        3 => CLOUD,
        45 | 48 => CLOUD_FOG,
        51..=67 | 80..=82 => CLOUD_RAIN,
        71..=77 | 85 | 86 => CLOUD_SNOW,
        95..=99 => CLOUD_LIGHTNING,
        _ => CLOUD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "current": {"time":"2026-11-06T14:15","temperature_2m":12.5,"apparent_temperature":10.0,
                "relative_humidity_2m":70,"is_day":1,"weather_code":61,"wind_speed_10m":9.0,"wind_gusts_10m":20.0},
            "hourly": {
                "time":["2026-11-06T13:00","2026-11-06T14:00","2026-11-06T15:00"],
                "temperature_2m":[1.0,2.0,3.0],"precipitation_probability":[0,10,20],
                "precipitation":[0.0,0.1,0.2],"weather_code":[0,61,3],"is_day":[1,1,0]
            },
            "daily": {
                "time":["2026-11-06"],"weather_code":[61],"temperature_2m_max":[14.0],"temperature_2m_min":[6.0],
                "precipitation_probability_max":[40],"uv_index_max":[2.0],
                "sunrise":["2026-11-06T07:05"],"sunset":["2026-11-06T16:40"]
            }
        })
    }

    #[test]
    fn forecast_is_read() {
        let forecast = parse_forecast(&sample(), "Traun").unwrap();
        assert_eq!(forecast.place, "Traun");
        assert_eq!(forecast.temperature, 12.5);
        assert_eq!(forecast.code, 61);
        assert!(forecast.is_day);
        assert_eq!(forecast.days[0].max, 14.0);
    }

    #[test]
    fn hours_start_at_the_current_hour() {
        let forecast = parse_forecast(&sample(), "").unwrap();
        assert_eq!(forecast.hours.len(), 2);
        assert_eq!(forecast.hours[0].temperature, 2.0);
        assert!(!forecast.hours[1].is_day);
    }

    #[test]
    fn missing_sections_are_an_error() {
        assert!(parse_forecast(&json!({"current":{}}), "").is_err());
    }

    #[test]
    fn every_code_maps_to_a_known_locale_key() {
        for code in 0..=100 {
            let key = condition_key(code);
            assert!(key.starts_with("weather.condition."));
        }
        assert_eq!(condition_key(95), "weather.condition.thunderstorm");
        assert_eq!(condition_key(1234), "weather.condition.unknown");
    }
}
