using System;
using System.IO;
using System.Linq;
using System.Windows.Threading;
using LifeHUD.Services;

namespace LifeHUD.ViewModels;

public sealed class NotesViewModel : ObservableObject
{
    private readonly string _path;
    private readonly DispatcherTimer _debounce;
    private readonly DispatcherTimer _fadeStatus;
    private string _text;
    private string _status = "";
    private bool _dirty;

    public NotesViewModel(string path)
    {
        _path = path;
        try { _text = File.Exists(path) ? File.ReadAllText(path) : ""; }
        catch (Exception ex) { Log.Write("Reading notes failed", ex); _text = ""; }

        _debounce = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(600) };
        _debounce.Tick += (_, _) => Flush();
        _fadeStatus = new DispatcherTimer { Interval = TimeSpan.FromSeconds(2.5) };
        _fadeStatus.Tick += (_, _) => { _fadeStatus.Stop(); Status = ""; };
    }

    public string Text
    {
        get => _text;
        set
        {
            if (!Set(ref _text, value ?? "")) return;
            _dirty = true;
            _debounce.Stop();
            _debounce.Start();
            Raise(nameof(WordCountText));
        }
    }

    /// <summary>Brief "Saved" confirmation that fades out.</summary>
    public string Status { get => _status; private set => Set(ref _status, value); }

    public string WordCountText
    {
        get
        {
            int words = _text.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries).Length;
            return words == 0 ? "" : words == 1 ? "1 word" : $"{words} words";
        }
    }

    public void Flush()
    {
        _debounce.Stop();
        if (!_dirty) return;
        try
        {
            JsonStore.WriteAtomic(_path, _text);
            _dirty = false;
            Status = "Saved";
            _fadeStatus.Stop();
            _fadeStatus.Start();
        }
        catch (Exception ex)
        {
            Log.Write("Saving notes failed", ex);
            Status = "Couldn't save";
        }
    }
}
