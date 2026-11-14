using System;
using System.Diagnostics;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using LifeHUD.Models;
using LifeHUD.Services;

namespace LifeHUD.ViewModels;

public sealed class MediaViewModel : ObservableObject
{
    private const string PlayGlyph = "", PauseGlyph = "";

    private readonly MediaService _service;
    private readonly DispatcherTimer _tick;

    private bool _hasMedia, _isPlaying, _isSpotify = true, _hasTimeline, _canPrevious, _canNext;
    private string _title = "Not playing", _artist = "Open Spotify to start listening", _sourceName = "SPOTIFY";
    private string _elapsedText = "0:00", _remainingText = "0:00", _progressTip = "";
    private double _progress;
    private BitmapSource? _artwork;
    private Brush _glowBrush = Brushes.Transparent;

    public MediaViewModel(MediaService service, SettingsService settings)
    {
        _service = service;
        _service.PreferSpotify = settings.Current.PreferSpotify;
        _service.StateChanged += (_, _) => Apply();
        settings.Changed += (_, key) =>
        {
            if (key != SettingKeys.Media) return;
            _service.PreferSpotify = settings.Current.PreferSpotify;
            _service.Reselect();
        };

        _tick = new DispatcherTimer(DispatcherPriority.Background) { Interval = TimeSpan.FromMilliseconds(500) };
        _tick.Tick += (_, _) => UpdateProgress();

        PlayPauseCommand = new RelayCommand(async () =>
        {
            if (!HasMedia) { OpenSpotify(); return; }
            IsPlaying = !IsPlaying; // optimistic; the real state follows within a few ms
            Raise(nameof(PlayPauseGlyph));
            await _service.PlayPauseAsync();
        });
        NextCommand = new RelayCommand(async () => await _service.NextAsync());
        PreviousCommand = new RelayCommand(async () => await _service.PreviousAsync());
        OpenSpotifyCommand = new RelayCommand(OpenSpotify);
    }

    public ICommand PlayPauseCommand { get; }
    public ICommand NextCommand { get; }
    public ICommand PreviousCommand { get; }
    public ICommand OpenSpotifyCommand { get; }

    public bool HasMedia { get => _hasMedia; private set => Set(ref _hasMedia, value); }
    public bool IsPlaying { get => _isPlaying; private set => Set(ref _isPlaying, value); }
    public bool IsSpotify { get => _isSpotify; private set => Set(ref _isSpotify, value); }
    public bool HasTimeline { get => _hasTimeline; private set => Set(ref _hasTimeline, value); }
    public bool CanPrevious { get => _canPrevious; private set => Set(ref _canPrevious, value); }
    public bool CanNext { get => _canNext; private set => Set(ref _canNext, value); }
    public string Title { get => _title; private set => Set(ref _title, value); }
    public string Artist { get => _artist; private set => Set(ref _artist, value); }
    public string SourceName { get => _sourceName; private set => Set(ref _sourceName, value); }
    public string ElapsedText { get => _elapsedText; private set => Set(ref _elapsedText, value); }
    public string RemainingText { get => _remainingText; private set => Set(ref _remainingText, value); }
    public string ProgressTip { get => _progressTip; private set => Set(ref _progressTip, value); }
    public double Progress { get => _progress; private set => Set(ref _progress, value); }
    public BitmapSource? Artwork { get => _artwork; private set { if (Set(ref _artwork, value)) Raise(nameof(HasArtwork)); } }
    public bool HasArtwork => _artwork != null;
    public Brush GlowBrush { get => _glowBrush; private set => Set(ref _glowBrush, value); }
    public string PlayPauseGlyph => IsPlaying ? PauseGlyph : PlayGlyph;

    private void Apply()
    {
        var s = _service.State;
        HasMedia = s.HasSession && (s.Title.Length > 0 || s.Artist.Length > 0);
        if (HasMedia)
        {
            Title = s.Title.Length > 0 ? s.Title : "Unknown track";
            Artist = s.Artist;
            SourceName = s.Source.ToUpperInvariant();
            IsSpotify = s.IsSpotify;
        }
        else
        {
            Title = "Not playing";
            Artist = "Open Spotify to start listening";
            SourceName = "SPOTIFY";
            IsSpotify = true;
        }
        IsPlaying = HasMedia && s.IsPlaying;
        CanPrevious = HasMedia && s.CanPrevious;
        CanNext = HasMedia && s.CanNext;
        Artwork = HasMedia ? s.Artwork : null;
        GlowBrush = MakeGlow(HasMedia ? s.ArtColor : null);
        HasTimeline = HasMedia && s.Duration > TimeSpan.FromSeconds(1);
        Raise(nameof(PlayPauseGlyph));

        if (IsPlaying && HasTimeline) _tick.Start(); else _tick.Stop();
        UpdateProgress();
    }

    private void UpdateProgress()
    {
        var s = _service.State;
        if (!HasTimeline) { Progress = 0; ProgressTip = ""; return; }
        var pos = s.Position;
        if (s.IsPlaying)
        {
            var elapsed = DateTimeOffset.Now - s.PositionUpdatedAt;
            if (elapsed > TimeSpan.Zero && elapsed < TimeSpan.FromHours(6)) pos += elapsed;
        }
        if (pos > s.Duration) pos = s.Duration;
        if (pos < TimeSpan.Zero) pos = TimeSpan.Zero;

        Progress = pos.TotalMilliseconds / s.Duration.TotalMilliseconds;
        ElapsedText = Format(pos);
        RemainingText = "-" + Format(s.Duration - pos);
        ProgressTip = $"{Format(pos)} / {Format(s.Duration)}";
    }

    private static string Format(TimeSpan t) =>
        t.TotalHours >= 1 ? $"{(int)t.TotalHours}:{t.Minutes:00}:{t.Seconds:00}" : $"{(int)t.TotalMinutes}:{t.Seconds:00}";

    private static Brush MakeGlow(Color? c)
    {
        if (c is not Color color) return Brushes.Transparent;
        var brush = new LinearGradientBrush
        {
            StartPoint = new Point(0, 0.5),
            EndPoint = new Point(1, 0.5),
            GradientStops =
            {
                new GradientStop(Color.FromArgb(0x66, color.R, color.G, color.B), 0),
                new GradientStop(Color.FromArgb(0x1C, color.R, color.G, color.B), 0.55),
                new GradientStop(Color.FromArgb(0x00, color.R, color.G, color.B), 1),
            },
        };
        brush.Freeze();
        return brush;
    }

    private static void OpenSpotify()
    {
        try { Process.Start(new ProcessStartInfo("spotify:") { UseShellExecute = true }); }
        catch
        {
            try { Process.Start(new ProcessStartInfo("https://open.spotify.com") { UseShellExecute = true }); }
            catch (Exception ex) { Log.Write("Could not launch Spotify", ex); }
        }
    }
}
