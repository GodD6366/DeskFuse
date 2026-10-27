using System.Text.Json;
using DeskFuse.Windows;

static void Check(bool condition, string message) { if (!condition) throw new Exception(message); }
static JsonElement Settings(params (string Device, string Display, double X, double Y)[] positions) =>
    JsonSerializer.SerializeToElement(new { placements = positions.Select(p => new { display = new { device_id = p.Device, display_id = p.Display }, x = p.X, y = p.Y }) });
static void NoOverlap(InputLayoutRect[] rects, string device, (double X, double Y) move) =>
    Check(!rects.Where(r => r.DeviceId == device).Any(a => rects.Where(r => r.DeviceId != device).Any(b => InputLayout.Overlaps(a, b, move.X, move.Y))), "preview overlaps another desktop");

var saved = Settings(("remote", "mac", -1920, 180));
Check(InputLayout.Placement(saved, "remote", "mac") == (-1920, 180), "nested DisplayKey was not read");
Check(InputLayout.Placement(saved, "remote", "other") is null, "wrong display matched");
Check(InputLayout.Placement(default, "remote", "mac") is null, "empty snapshot must have no position");
Console.WriteLine("PASS: runtime placement schema, negative/fractional coordinates and missing display");

InputLayoutRect[] two = [new("local", "win", 0, 0, 2560, 1440), new("remote", "mac", 2560, 0, 1920, 1080)];
var left = InputLayout.SnapMove(two, "remote", -700, 170.5);
Check(left == (-4480, 180), "leftward overlap did not preview the left seam");
NoOverlap(two, "remote", left);
Check(InputLayout.Confirms(saved, two, "remote", left.X, left.Y), "saved left layout not confirmed");
Check(!InputLayout.Confirms(Settings(("remote", "mac", 2560, 0)), two, "remote", left.X, left.Y), "stale heartbeat confirmed move");
Check(!InputLayout.Confirms(Settings(), two, "remote", left.X, left.Y), "missing placement confirmed move");
Check(InputLayout.SnapMove(two, "remote", 0, 0) == (0, 0), "click without movement moved the screen");
Console.WriteLine("PASS: immediate leftward avoidance, save confirmation and stale refresh rejection");

var onLeft = two.Select(r => r.DeviceId == "remote" ? r with { X = -1920 } : r).ToArray();
var right = InputLayout.SnapMove(onLeft, "remote", 700, 0);
Check(right == (4480, 0), "rightward overlap did not preview right seam");
NoOverlap(onLeft, "remote", right);
var above = two.Select(r => r.DeviceId == "remote" ? r with { X = 0, Y = -1080 } : r).ToArray();
var down = InputLayout.SnapMove(above, "remote", 0, 500);
Check(down == (0, 2520), "downward overlap did not preview bottom seam");
var below = two.Select(r => r.DeviceId == "remote" ? r with { X = 0, Y = 1440 } : r).ToArray();
var up = InputLayout.SnapMove(below, "remote", 0, -500);
Check(up == (0, -2520), "upward overlap did not preview top seam");
NoOverlap(above, "remote", down); NoOverlap(below, "remote", up);
Console.WriteLine("PASS: avoidance works in all four directions");

InputLayoutRect[] group = [..two, new("remote", "second", 4480, 0, 1280, 720), new("third", "extra", -1200, 0, 1200, 900)];
var move = InputLayout.SnapMove(group, "remote", -800, 90);
NoOverlap(group, "remote", move);
var moving = group.Where(r => r.DeviceId == "remote").ToArray();
var partial = Settings(("remote", "mac", moving[0].X + move.X, moving[0].Y + move.Y), ("remote", "second", moving[1].X, moving[1].Y));
Check(!InputLayout.Confirms(partial, group, "remote", move.X, move.Y), "partially moved native desktop confirmed");
var complete = Settings(moving.Select(r => (r.DeviceId, r.DisplayId, r.X + move.X, r.Y + move.Y)).ToArray());
Check(InputLayout.Confirms(complete, group, "remote", move.X, move.Y), "complete desktop move not confirmed");
Console.WriteLine("PASS: whole-device movement, third-screen avoidance and all-screen confirmation");

// Synthetic drag paths only: validate preview positions and that release at an
// already resolved seam is a fixed point (the runtime's nearest-seam behavior).
foreach (var rects in new[] { two, group })
for (int x = -6000; x <= 2400; x += 120)
for (int y = -1800; y <= 1800; y += 120)
{
    var preview = InputLayout.SnapMove(rects, "remote", x, y);
    NoOverlap(rects, "remote", preview);
    Check(InputLayout.SnapMove(rects, "remote", preview.X, preview.Y) == preview, "release changed a valid preview seam");
}
var endpoint = InputLayout.SnapMove(two, "remote", -820, 175);
for (int i = 0; i < 100; i++) InputLayout.SnapMove(two, "remote", -i * 8.2, i * 1.75);
Check(InputLayout.SnapMove(two, "remote", -820, 175) == endpoint, "snapped history changed absolute pointer displacement");
Console.WriteLine("PASS: 4402 drag positions are non-overlapping, stable on release and independent of previous preview");
