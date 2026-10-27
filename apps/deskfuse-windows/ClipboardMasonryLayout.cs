using System.Collections.Specialized;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;

namespace DeskFuse.Windows;

/// Two-column masonry with viewport-only realization, not two eager StackPanels.
/// Unseen items use an estimate; measured heights are retained until data/width
/// changes. ItemsRepeater recycles everything not requested in this measure.
public sealed class ClipboardMasonryLayout : VirtualizingLayout
{
    private sealed class State
    {
        public double Width;
        public readonly Dictionary<int, double> Heights = [];
        public readonly List<(UIElement Element, Rect Bounds)> Realized = [];
    }

    protected override void InitializeForContextCore(VirtualizingLayoutContext context) => context.LayoutState = new State();
    protected override void UninitializeForContextCore(VirtualizingLayoutContext context) => context.LayoutState = null;

    protected override void OnItemsChangedCore(VirtualizingLayoutContext context, object source, NotifyCollectionChangedEventArgs args)
    {
        if (context.LayoutState is State state) state.Heights.Clear();
        InvalidateMeasure();
    }

    protected override Size MeasureOverride(VirtualizingLayoutContext context, Size availableSize)
    {
        var state = (State)context.LayoutState;
        double width = double.IsFinite(availableSize.Width) ? Math.Max(0, availableSize.Width) : 320;
        if (Math.Abs(state.Width - width) > 0.5) { state.Heights.Clear(); state.Width = width; }
        state.Realized.Clear();
        const double gap = 12;
        double columnWidth = Math.Max(0, (width - gap) / 2);
        double[] bottom = [0, 0];
        var viewport = context.RealizationRect;
        for (int index = 0; index < context.ItemCount; index++)
        {
            int column = bottom[0] <= bottom[1] ? 0 : 1;
            double height = state.Heights.GetValueOrDefault(index, 140);
            var bounds = new Rect(column * (columnWidth + gap), bottom[column], columnWidth, height);
            if (bounds.Bottom >= viewport.Top && bounds.Top <= viewport.Bottom)
            {
                var element = context.GetOrCreateElementAt(index);
                element.Measure(new Size(columnWidth, double.PositiveInfinity));
                height = Math.Max(1, element.DesiredSize.Height);
                state.Heights[index] = height;
                bounds.Height = height;
                state.Realized.Add((element, bounds));
            }
            bottom[column] += height + gap;
        }
        return new Size(width, Math.Max(0, bottom.Max() - gap));
    }

    protected override Size ArrangeOverride(VirtualizingLayoutContext context, Size finalSize)
    {
        foreach (var (element, bounds) in ((State)context.LayoutState).Realized) element.Arrange(bounds);
        return finalSize;
    }
}
