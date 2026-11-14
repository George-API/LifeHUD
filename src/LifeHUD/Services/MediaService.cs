using System;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Windows.Media.Control;

namespace LifeHUD.Services;

public sealed record MediaState(
    bool HasSession,
    string Title,
    string Artist,
    string Source,
    bool IsSpotify,
    bool IsPlaying,
    TimeSpan Position,
    TimeSpan Duration,
    DateTimeOffset PositionUpdatedAt,
    BitmapSource? Artwork,
    Color? ArtColor,
    bool CanPrevious,
    bool CanNext)
{
    public static readonly MediaState Empty = new(false, "", "", "", false, false, TimeSpan.Zero, TimeSpan.Zero,
        DateTimeOffset.MinValue, null, null, false, false);
}

/// <summary>
/// Reads "now playing" from Windows' System Media Transport Controls. Spotify desktop publishes its
/// track, artwork, timeline and transport controls there, so no OAuth / Web API is needed, and it keeps
/// working offline. Other players are used as a fallback.
/// </summary>
public sealed class MediaService : IDisposable
{
    private readonly Dispatcher _ui;
    private GlobalSystemMediaTransportControlsSessionManager? _manager;
    private GlobalSystemMediaTransportControlsSession? _session;

    private string _title = "", _artist = "";
    private string? _artKey;
    private BitmapSource? _art;
    private Color? _artColor;

    public MediaService(Dispatcher ui) => _ui = ui;

    public bool PreferSpotify { get; set; } = true;
    public MediaState State { get; private set; } = MediaState.Empty;
    public event EventHandler? StateChanged;

    public async Task StartAsync()
    {
        try
        {
            _manager = await GlobalSystemMediaTransportControlsSessionManager.RequestAsync();
            _manager.SessionsChanged += (_, _) => Post(SelectSession);
            _manager.CurrentSessionChanged += (_, _) => Post(SelectSession);
            SelectSession();
        }
        catch (Exception ex)
        {
            Log.Write("Media session manager unavailable", ex);
        }
    }

    public void Reselect() => SelectSession();

    private void Post(Action action) => _ui.BeginInvoke(action);

    private void SelectSession()
    {
        if (_manager is null) return;
        GlobalSystemMediaTransportControlsSession? pick = null;
        try
        {
            var sessions = _manager.GetSessions();
            if (PreferSpotify) pick = sessions.FirstOrDefault(s => IsSpotify(s.SourceAppUserModelId));
            pick ??= _manager.GetCurrentSession();
        }
        catch (Exception ex)
        {
            Log.Write("Enumerating media sessions failed", ex);
        }

        // Always rehook: a player that restarts produces a new session object with the same app id.
        Unhook(_session);
        _session = pick;
        Hook(_session);
        _ = RefreshAllAsync();
    }

    private void Hook(GlobalSystemMediaTransportControlsSession? s)
    {
        if (s is null) return;
        s.MediaPropertiesChanged += OnMediaPropertiesChanged;
        s.PlaybackInfoChanged += OnPlaybackChanged;
        s.TimelinePropertiesChanged += OnTimelineChanged;
    }

    private void Unhook(GlobalSystemMediaTransportControlsSession? s)
    {
        if (s is null) return;
        try
        {
            s.MediaPropertiesChanged -= OnMediaPropertiesChanged;
            s.PlaybackInfoChanged -= OnPlaybackChanged;
            s.TimelinePropertiesChanged -= OnTimelineChanged;
        }
        catch { /* session already gone */ }
    }

    private void OnMediaPropertiesChanged(GlobalSystemMediaTransportControlsSession sender, MediaPropertiesChangedEventArgs e) =>
        Post(() => { if (IsCurrent(sender)) _ = RefreshAllAsync(); });

    private void OnPlaybackChanged(GlobalSystemMediaTransportControlsSession sender, PlaybackInfoChangedEventArgs e) =>
        Post(() => { if (IsCurrent(sender)) Publish(); });

    private void OnTimelineChanged(GlobalSystemMediaTransportControlsSession sender, TimelinePropertiesChangedEventArgs e) =>
        Post(() => { if (IsCurrent(sender)) Publish(); });

    private bool IsCurrent(GlobalSystemMediaTransportControlsSession sender)
    {
        if (_session is null) return false;
        if (ReferenceEquals(sender, _session)) return true;
        try { return sender.SourceAppUserModelId == _session.SourceAppUserModelId; } catch { return false; }
    }

    private async Task RefreshAllAsync()
    {
        var s = _session;
        if (s is null)
        {
            _title = _artist = "";
            _art = null; _artColor = null; _artKey = null;
            Publish();
            return;
        }

        try
        {
            var props = await s.TryGetMediaPropertiesAsync();
            if (!ReferenceEquals(s, _session) || props is null) return;
            _title = props.Title ?? "";
            _artist = string.IsNullOrWhiteSpace(props.Artist) ? props.AlbumArtist ?? "" : props.Artist;

            string key = $"{_title}\u001f{_artist}";
            if (key != _artKey) { _art = null; _artColor = null; }
            Publish(); // show text immediately; artwork follows

            if (key != _artKey && props.Thumbnail is not null)
            {
                var art = await DecodeAsync(props.Thumbnail);
                if (!ReferenceEquals(s, _session) || art is null) return;
                _art = art;
                _artColor = ArtColor.Extract(art);
                _artKey = key;
            }
        }
        catch (Exception ex)
        {
            Log.Write("Reading media properties failed", ex);
        }
        Publish();
    }

    private static async Task<BitmapSource?> DecodeAsync(Windows.Storage.Streams.IRandomAccessStreamReference thumb)
    {
        try
        {
            using var ras = await thumb.OpenReadAsync();
            using var input = ras.AsStreamForRead();
            var ms = new MemoryStream();
            await input.CopyToAsync(ms);
            if (ms.Length == 0) return null;
            ms.Position = 0;
            var bmp = new BitmapImage();
            bmp.BeginInit();
            bmp.CacheOption = BitmapCacheOption.OnLoad;
            bmp.DecodePixelWidth = 192;
            bmp.StreamSource = ms;
            bmp.EndInit();
            bmp.Freeze();
            return bmp;
        }
        catch (Exception ex)
        {
            Log.Write("Decoding album art failed", ex);
            return null;
        }
    }

    private void Publish()
    {
        var s = _session;
        if (s is null)
        {
            State = MediaState.Empty;
            StateChanged?.Invoke(this, EventArgs.Empty);
            return;
        }

        bool playing = false, canPrev = false, canNext = false;
        TimeSpan pos = TimeSpan.Zero, dur = TimeSpan.Zero;
        DateTimeOffset updated = DateTimeOffset.Now;
        string appId = "";
        try
        {
            appId = s.SourceAppUserModelId ?? "";
            var info = s.GetPlaybackInfo();
            playing = info.PlaybackStatus == GlobalSystemMediaTransportControlsSessionPlaybackStatus.Playing;
            canPrev = info.Controls.IsPreviousEnabled;
            canNext = info.Controls.IsNextEnabled;
            var t = s.GetTimelineProperties();
            pos = t.Position;
            dur = t.EndTime - t.StartTime;
            updated = t.LastUpdatedTime;
        }
        catch { /* session vanished between calls */ }

        State = new MediaState(true, _title, _artist, FriendlySource(appId), IsSpotify(appId), playing,
            pos, dur, updated, _art, _artColor, canPrev, canNext);
        StateChanged?.Invoke(this, EventArgs.Empty);
    }

    public async Task PlayPauseAsync()
    {
        if (_session is null) return;
        try { await _session.TryTogglePlayPauseAsync(); } catch (Exception ex) { Log.Write("Play/pause failed", ex); }
    }

    public async Task NextAsync()
    {
        if (_session is null) return;
        try { await _session.TrySkipNextAsync(); } catch (Exception ex) { Log.Write("Skip next failed", ex); }
    }

    public async Task PreviousAsync()
    {
        if (_session is null) return;
        try { await _session.TrySkipPreviousAsync(); } catch (Exception ex) { Log.Write("Skip previous failed", ex); }
    }

    private static bool IsSpotify(string? appId) => appId?.Contains("spotify", StringComparison.OrdinalIgnoreCase) == true;

    private static string FriendlySource(string appId)
    {
        string id = appId.ToLowerInvariant();
        if (id.Contains("spotify")) return "Spotify";
        if (id.Contains("chrome")) return "Chrome";
        if (id.Contains("msedge")) return "Edge";
        if (id.Contains("firefox")) return "Firefox";
        if (id.Contains("zunemusic") || id.Contains("media.player")) return "Media Player";
        if (id.Contains("applemusic") || id.Contains("itunes")) return "Apple Music";
        return "Now Playing";
    }

    public void Dispose()
    {
        Unhook(_session);
        _session = null;
    }
}

/// <summary>Picks a representative, slightly saturated colour from album art for the widget glow.</summary>
internal static class ArtColor
{
    public static Color? Extract(BitmapSource source)
    {
        try
        {
            var small = new TransformedBitmap(source, new ScaleTransform(16.0 / source.PixelWidth, 16.0 / source.PixelHeight));
            var bgra = new FormatConvertedBitmap(small, PixelFormats.Bgra32, null, 0);
            int w = bgra.PixelWidth, h = bgra.PixelHeight;
            var px = new byte[w * h * 4];
            bgra.CopyPixels(px, w * 4, 0);

            double r = 0, g = 0, b = 0, total = 0;
            for (int i = 0; i < px.Length; i += 4)
            {
                byte pb = px[i], pg = px[i + 1], pr = px[i + 2];
                int max = Math.Max(pr, Math.Max(pg, pb)), min = Math.Min(pr, Math.Min(pg, pb));
                double sat = max == 0 ? 0 : (max - min) / (double)max;
                double lum = max / 255.0;
                // Favour vivid, mid-bright pixels over greys, blacks and blown highlights.
                double weight = 0.05 + sat * sat * (1 - Math.Abs(lum - 0.6));
                r += pr * weight; g += pg * weight; b += pb * weight; total += weight;
            }
            if (total <= 0) return null;
            var avg = Color.FromRgb((byte)(r / total), (byte)(g / total), (byte)(b / total));
            int hi = Math.Max(avg.R, Math.Max(avg.G, avg.B)), lo = Math.Min(avg.R, Math.Min(avg.G, avg.B));
            // Near-grey art would only muddy the card; skip the glow.
            if (hi == 0 || (hi - lo) / (double)hi < 0.18) return null;
            return avg;
        }
        catch
        {
            return null;
        }
    }
}
