using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;
using System.Windows.Input;
using System.Windows.Threading;
using LifeHUD.Models;
using LifeHUD.Services;

namespace LifeHUD.ViewModels;

public sealed class TaskItemViewModel : ObservableObject
{
    public TaskItemViewModel(TaskItem model) => Model = model;

    public TaskItem Model { get; }
    public string Title => Model.Title;

    public bool IsDone
    {
        get => Model.IsDone;
        set
        {
            if (Model.IsDone == value) return;
            Model.IsDone = value;
            Model.CompletedAt = value ? DateTime.Now : null;
            Raise(nameof(IsDone));
        }
    }
}

/// <summary>
/// A daily checklist: unfinished tasks carry over, tasks completed on a previous day are cleared
/// automatically at the first moment of a new day.
/// </summary>
public sealed class TasksViewModel : ObservableObject
{
    private readonly string _path;
    private string _newTaskText = "";

    public TasksViewModel(string path, ClockViewModel clock)
    {
        _path = path;
        var items = JsonStore.Load(path, () => new List<TaskItem>());
        foreach (var t in items.Where(t => !string.IsNullOrWhiteSpace(t.Title)).OrderBy(t => t.IsDone))
            Track(new TaskItemViewModel(t), Items.Count);

        AddTaskCommand = new RelayCommand(Add);
        RemoveTaskCommand = new RelayCommand(p => { if (p is TaskItemViewModel vm) Remove(vm); });
        ClearCompletedCommand = new RelayCommand(ClearCompleted);

        clock.DayChanged += (_, _) => Rollover();
        Rollover();
    }

    public ObservableCollection<TaskItemViewModel> Items { get; } = new();

    public ICommand AddTaskCommand { get; }
    public ICommand RemoveTaskCommand { get; }
    public ICommand ClearCompletedCommand { get; }

    public string NewTaskText { get => _newTaskText; set => Set(ref _newTaskText, value); }

    public int Total => Items.Count;
    public int Completed => Items.Count(i => i.IsDone);
    public int Remaining => Total - Completed;
    public double Progress => Total == 0 ? 0 : (double)Completed / Total;
    public bool IsEmpty => Total == 0;
    public bool HasCompleted => Completed > 0;
    public string RatioText => Total == 0 ? "0" : $"{Completed}/{Total}";

    public string SummaryText => Total switch
    {
        0 => "Nothing planned yet",
        _ when Remaining == 0 => "All done. Nice work.",
        _ => $"{Completed} of {Total} done",
    };

    private void Add()
    {
        string text = NewTaskText.Trim();
        if (text.Length == 0) return;
        var vm = new TaskItemViewModel(new TaskItem { Title = text });
        // New tasks go to the end of the open list, above anything already completed.
        int firstDone = IndexOfFirstDone();
        Track(vm, firstDone < 0 ? Items.Count : firstDone);
        NewTaskText = "";
        Changed();
    }

    private void Remove(TaskItemViewModel vm)
    {
        vm.PropertyChanged -= OnItemChanged;
        Items.Remove(vm);
        Changed();
    }

    private void ClearCompleted()
    {
        foreach (var vm in Items.Where(i => i.IsDone).ToList()) { vm.PropertyChanged -= OnItemChanged; Items.Remove(vm); }
        Changed();
    }

    private void Rollover()
    {
        var today = DateTime.Today;
        var stale = Items.Where(i => i.IsDone && (i.Model.CompletedAt ?? DateTime.MinValue).Date < today).ToList();
        if (stale.Count == 0) return;
        foreach (var vm in stale) { vm.PropertyChanged -= OnItemChanged; Items.Remove(vm); }
        Changed();
    }

    private void Track(TaskItemViewModel vm, int index)
    {
        vm.PropertyChanged += OnItemChanged;
        Items.Insert(index, vm);
    }

    private void OnItemChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e)
    {
        if (sender is not TaskItemViewModel vm || e.PropertyName != nameof(TaskItemViewModel.IsDone)) return;
        Changed();

        if (vm.IsDone)
        {
            // Let the completion animation play in place, then settle the item below open tasks.
            var settle = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(750) };
            settle.Tick += (_, _) =>
            {
                settle.Stop();
                int from = Items.IndexOf(vm);
                if (from < 0 || !vm.IsDone) return;
                int to = Items.Count - 1;
                if (from != to) { Items.Move(from, to); Save(); }
            };
            settle.Start();
        }
        else
        {
            int from = Items.IndexOf(vm);
            int firstDone = IndexOfFirstDone();
            if (from >= 0 && firstDone >= 0 && firstDone < from) { Items.Move(from, firstDone); Save(); }
        }
    }

    private int IndexOfFirstDone()
    {
        for (int i = 0; i < Items.Count; i++) if (Items[i].IsDone) return i;
        return -1;
    }

    private void Changed()
    {
        Save();
        Raise(nameof(Total), nameof(Completed), nameof(Remaining), nameof(Progress), nameof(IsEmpty),
              nameof(HasCompleted), nameof(RatioText), nameof(SummaryText));
    }

    private void Save()
    {
        try { JsonStore.Save(_path, Items.Select(i => i.Model).ToList()); }
        catch (Exception ex) { Log.Write("Saving tasks failed", ex); }
    }
}
