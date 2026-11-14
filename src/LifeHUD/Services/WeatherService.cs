using System;
using System.Globalization;
using System.Linq;
using System.Net.Http;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using LifeHUD.Models;
using Windows.Devices.Geolocation;

namespace LifeHUD.Services;

/// <summary>Open-Meteo (no API key) for forecasts and geocoding; Windows location for "automatic".</summary>
public sealed class WeatherService
{
    private static readonly HttpClient Http = CreateClient();

    private static HttpClient CreateClient()
    {
        var client = new HttpClient { Timeout = TimeSpan.FromSeconds(15) };
        client.DefaultRequestHeaders.UserAgent.ParseAdd("LifeHUD/0.1");
        return client;
    }

    public async Task<GeoLocation?> GeocodeAsync(string city, CancellationToken ct = default)
    {
        var url = $"https://geocoding-api.open-meteo.com/v1/search?name={Uri.EscapeDataString(city)}&count=1&language=en&format=json";
        using var doc = JsonDocument.Parse(await Http.GetStringAsync(url, ct));
        if (!doc.RootElement.TryGetProperty("results", out var results) || results.GetArrayLength() == 0) return null;
        var r = results[0];
        string name = r.GetProperty("name").GetString() ?? city;
        if (r.TryGetProperty("country", out var country) && country.GetString() is string c && c.Length > 0)
            name = $"{name}, {c}";
        return new GeoLocation(r.GetProperty("latitude").GetDouble(), r.GetProperty("longitude").GetDouble(), name);
    }

    /// <summary>Windows location services. Returns null when disabled or denied.</summary>
    public async Task<GeoLocation?> GetDeviceLocationAsync()
    {
        try
        {
            var access = await Geolocator.RequestAccessAsync();
            if (access != GeolocationAccessStatus.Allowed) return null;
            var locator = new Geolocator { DesiredAccuracy = PositionAccuracy.Default };
            var pos = await locator.GetGeopositionAsync(TimeSpan.FromMinutes(30), TimeSpan.FromSeconds(10));
            var p = pos.Coordinate.Point.Position;
            return new GeoLocation(p.Latitude, p.Longitude, "My Location");
        }
        catch (Exception ex)
        {
            Log.Write("Device location unavailable", ex);
            return null;
        }
    }

    public async Task<WeatherSnapshot> FetchAsync(GeoLocation loc, bool celsius, string locationKey, CancellationToken ct = default)
    {
        string F(double v) => v.ToString("0.####", CultureInfo.InvariantCulture);
        var url = "https://api.open-meteo.com/v1/forecast" +
                  $"?latitude={F(loc.Latitude)}&longitude={F(loc.Longitude)}" +
                  "&current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,is_day,wind_speed_10m" +
                  "&hourly=temperature_2m,weather_code,is_day" +
                  "&daily=weather_code,temperature_2m_max,temperature_2m_min" +
                  "&timezone=auto&forecast_days=6" +
                  (celsius ? "&temperature_unit=celsius&wind_speed_unit=kmh" : "&temperature_unit=fahrenheit&wind_speed_unit=mph");

        using var doc = JsonDocument.Parse(await Http.GetStringAsync(url, ct));
        var root = doc.RootElement;
        var cur = root.GetProperty("current");

        var snap = new WeatherSnapshot
        {
            LocationName = loc.Name,
            LocationKey = locationKey,
            Celsius = celsius,
            FetchedAt = DateTimeOffset.Now,
            Temperature = cur.GetProperty("temperature_2m").GetDouble(),
            FeelsLike = cur.GetProperty("apparent_temperature").GetDouble(),
            Humidity = cur.GetProperty("relative_humidity_2m").GetDouble(),
            WindSpeed = cur.GetProperty("wind_speed_10m").GetDouble(),
            Code = cur.GetProperty("weather_code").GetInt32(),
            IsDay = cur.GetProperty("is_day").GetInt32() == 1,
        };

        // Hourly: start from the current hour in the location's own timezone.
        var nowLocal = ParseTime(cur.GetProperty("time").GetString());
        var hourStart = new DateTime(nowLocal.Year, nowLocal.Month, nowLocal.Day, nowLocal.Hour, 0, 0);
        var hourly = root.GetProperty("hourly");
        var hTimes = hourly.GetProperty("time");
        var hTemps = hourly.GetProperty("temperature_2m");
        var hCodes = hourly.GetProperty("weather_code");
        var hDay = hourly.GetProperty("is_day");
        for (int i = 0; i < hTimes.GetArrayLength() && snap.Hours.Count < 24; i++)
        {
            var t = ParseTime(hTimes[i].GetString());
            if (t < hourStart || hTemps[i].ValueKind != JsonValueKind.Number) continue;
            snap.Hours.Add(new HourPoint
            {
                Time = t,
                Temperature = hTemps[i].GetDouble(),
                Code = hCodes[i].ValueKind == JsonValueKind.Number ? hCodes[i].GetInt32() : 3,
                IsDay = hDay[i].ValueKind != JsonValueKind.Number || hDay[i].GetInt32() == 1,
            });
        }

        var daily = root.GetProperty("daily");
        var dTimes = daily.GetProperty("time");
        var dCodes = daily.GetProperty("weather_code");
        var dMax = daily.GetProperty("temperature_2m_max");
        var dMin = daily.GetProperty("temperature_2m_min");
        for (int i = 0; i < dTimes.GetArrayLength(); i++)
        {
            if (dMax[i].ValueKind != JsonValueKind.Number || dMin[i].ValueKind != JsonValueKind.Number) continue;
            snap.Days.Add(new DayPoint
            {
                Date = DateTime.ParseExact(dTimes[i].GetString()!, "yyyy-MM-dd", CultureInfo.InvariantCulture),
                Code = dCodes[i].ValueKind == JsonValueKind.Number ? dCodes[i].GetInt32() : 3,
                High = dMax[i].GetDouble(),
                Low = dMin[i].GetDouble(),
            });
        }

        if (snap.Days.Count == 0) throw new InvalidOperationException("Forecast response had no daily data");
        return snap;
    }

    private static DateTime ParseTime(string? s) =>
        DateTime.ParseExact(s ?? throw new FormatException("missing time"), "yyyy-MM-dd'T'HH:mm", CultureInfo.InvariantCulture);
}
