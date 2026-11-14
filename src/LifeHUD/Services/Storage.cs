using System;
using System.IO;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace LifeHUD.Services;

public static class AppPaths
{
    public static readonly string Root = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "LifeHUD");

    public static readonly string Settings = Path.Combine(Root, "settings.json");
    public static readonly string Tasks = Path.Combine(Root, "tasks.json");
    public static readonly string Notes = Path.Combine(Root, "notes.md");
    public static readonly string WeatherCache = Path.Combine(Root, "weather-cache.json");
    public static readonly string Log = Path.Combine(Root, "lifehud.log");

    public static void Ensure() => Directory.CreateDirectory(Root);
}

public static class JsonStore
{
    private static readonly JsonSerializerOptions Options = new()
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() },
    };

    public static T Load<T>(string path, Func<T> fallback)
    {
        try
        {
            if (!File.Exists(path)) return fallback();
            return JsonSerializer.Deserialize<T>(File.ReadAllText(path), Options) ?? fallback();
        }
        catch (Exception ex)
        {
            Log.Write($"Could not read {Path.GetFileName(path)}; keeping a copy and starting fresh", ex);
            try { File.Copy(path, path + ".corrupt", overwrite: true); } catch { }
            return fallback();
        }
    }

    /// <summary>Atomic write: a crash mid-save never leaves a truncated file behind.</summary>
    public static void Save<T>(string path, T value) => WriteAtomic(path, JsonSerializer.Serialize(value, Options));

    public static void WriteAtomic(string path, string contents)
    {
        var tmp = path + ".tmp";
        File.WriteAllText(tmp, contents);
        File.Move(tmp, path, overwrite: true);
    }
}

public static class Log
{
    private static readonly object Gate = new();

    public static void Write(string message, Exception? ex = null)
    {
        try
        {
            lock (Gate)
            {
                var info = new FileInfo(AppPaths.Log);
                if (info.Exists && info.Length > 512 * 1024) info.Delete();
                File.AppendAllText(AppPaths.Log,
                    $"{DateTime.Now:yyyy-MM-dd HH:mm:ss} {message}{(ex is null ? "" : " :: " + ex)}{Environment.NewLine}");
            }
        }
        catch { /* logging must never throw */ }
    }
}
