using System;
using System.Collections.Generic;

namespace LifeHUD.Models;

public enum WeatherCondition { Clear, PartlyCloudy, Cloudy, Fog, Drizzle, Rain, Snow, Thunder }

public sealed record GeoLocation(double Latitude, double Longitude, string Name);

public sealed class WeatherSnapshot
{
    public string LocationName { get; set; } = string.Empty;
    /// <summary>Identifies which location source produced this (city query, or "device").</summary>
    public string LocationKey { get; set; } = string.Empty;
    public bool Celsius { get; set; }
    public DateTimeOffset FetchedAt { get; set; }

    public double Temperature { get; set; }
    public double FeelsLike { get; set; }
    public double Humidity { get; set; }
    public double WindSpeed { get; set; }
    public int Code { get; set; }
    public bool IsDay { get; set; } = true;

    public List<HourPoint> Hours { get; set; } = new();
    public List<DayPoint> Days { get; set; } = new();
}

public sealed class HourPoint
{
    public DateTime Time { get; set; }
    public double Temperature { get; set; }
    public int Code { get; set; }
    public bool IsDay { get; set; } = true;
}

public sealed class DayPoint
{
    public DateTime Date { get; set; }
    public int Code { get; set; }
    public double High { get; set; }
    public double Low { get; set; }
}

/// <summary>WMO weather interpretation codes, as used by Open-Meteo.</summary>
public static class WeatherCodes
{
    public static WeatherCondition ToCondition(int code) => code switch
    {
        0 => WeatherCondition.Clear,
        1 or 2 => WeatherCondition.PartlyCloudy,
        3 => WeatherCondition.Cloudy,
        45 or 48 => WeatherCondition.Fog,
        >= 51 and <= 57 => WeatherCondition.Drizzle,
        (>= 61 and <= 67) or (>= 80 and <= 82) => WeatherCondition.Rain,
        (>= 71 and <= 77) or 85 or 86 => WeatherCondition.Snow,
        >= 95 => WeatherCondition.Thunder,
        _ => WeatherCondition.Cloudy,
    };

    public static string Describe(int code, bool isDay) => code switch
    {
        0 => isDay ? "Sunny" : "Clear",
        1 => isDay ? "Mostly sunny" : "Mostly clear",
        2 => "Partly cloudy",
        3 => "Cloudy",
        45 or 48 => "Fog",
        51 or 53 or 55 => "Drizzle",
        56 or 57 => "Freezing drizzle",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        66 or 67 => "Freezing rain",
        71 => "Light snow",
        73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80 => "Showers",
        81 => "Rain showers",
        82 => "Heavy showers",
        85 or 86 => "Snow showers",
        95 => "Thunderstorms",
        96 or 99 => "Storms with hail",
        _ => "—",
    };
}
