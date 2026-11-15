// The lazer oracle (ADRs 0026, 0032): writes lazer's expected results for .osu files as JSON.
// Usage: LazerOracle <output folder> <input folder>...
// Every .osu file under each input folder gets <output folder>/<input folder name>/<relative path>.json.
// Output is written to a temporary folder and moved into place only when every file succeeded.

using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using osu.Game.Audio;
using osu.Game.Beatmaps;
using osu.Game.IO;
using osu.Game.Rulesets.Mods;
using osu.Game.Rulesets.Objects;
using osu.Game.Rulesets.Objects.Legacy;
using osu.Game.Rulesets.Objects.Types;
using osu.Game.Rulesets.Osu.Objects;
using Decoder = osu.Game.Beatmaps.Formats.Decoder;

public static class Program
{
    // Bumped whenever the shape of the JSON changes.
    private const int ResultFormatVersion = 1;

    public static int Main(string[] args)
    {
        if (args.Length < 2)
        {
            Console.Error.WriteLine("usage: LazerOracle <output folder> <input folder>...");
            return 2;
        }

        string output = Path.GetFullPath(args[0]);
        string staging = output + ".staging-" + Guid.NewGuid().ToString("N");
        string lazerVersion = typeof(Beatmap).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion ?? "unknown";

        try
        {
            int count = 0;
            foreach (string input in args.Skip(1).Select(Path.GetFullPath))
            {
                string group = Path.GetFileName(input.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar));
                foreach (string file in Directory.EnumerateFiles(input, "*.osu", SearchOption.AllDirectories).OrderBy(f => f, StringComparer.Ordinal))
                {
                    string relative = Path.GetRelativePath(input, file).Replace('\\', '/');
                    string target = Path.Combine(staging, group, relative + ".json");
                    Directory.CreateDirectory(Path.GetDirectoryName(target)!);
                    File.WriteAllBytes(target, Describe(File.ReadAllBytes(file), group + "/" + relative, lazerVersion));
                    count++;
                }
            }

            if (Directory.Exists(output))
                Directory.Delete(output, recursive: true);
            Directory.Move(staging, output);
            Console.WriteLine($"wrote {count} results to {output}");
            return 0;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"error: {ex.Message}");
            if (Directory.Exists(staging))
                Directory.Delete(staging, recursive: true);
            return 1;
        }
    }

    private static byte[] Describe(byte[] bytes, string name, string lazerVersion)
    {
        int mode = ReadMode(bytes);

        using var stream = new MemoryStream(bytes);
        using var reader = new LineBufferedReader(stream);
        var decoded = Decoder.GetDecoder<Beatmap>(reader).Decode(reader);

        // ADR 0032 points 2 and 3: the map must be read in its own mode.
        if (decoded.BeatmapInfo.Ruleset.OnlineID != mode)
            throw new InvalidOperationException($"{name}: Mode line says {mode}, lazer read ruleset {decoded.BeatmapInfo.Ruleset.OnlineID}");

        var playable = new FlatWorkingBeatmap(decoded).GetPlayableBeatmap(decoded.BeatmapInfo.Ruleset, Array.Empty<Mod>(), CancellationToken.None);

        var buffer = new MemoryStream();
        using (var w = new Utf8JsonWriter(buffer, new JsonWriterOptions { Indented = true, NewLine = "\n" }))
        {
            w.WriteStartObject();
            w.WritePropertyName("header");
            w.WriteStartObject();
            w.WriteNumber("result_format_version", ResultFormatVersion);
            w.WriteString("input", name);
            w.WriteString("input_sha256", Convert.ToHexStringLower(SHA256.HashData(bytes)));
            w.WriteString("lazer_version", lazerVersion);
            w.WriteNumber("mode", mode);
            w.WriteString("ruleset", decoded.BeatmapInfo.Ruleset.ShortName);
            w.WriteNumber("format_version", decoded.BeatmapVersion);
            w.WriteEndObject();

            w.WritePropertyName("hit_objects");
            w.WriteStartArray();
            foreach (var obj in playable.HitObjects)
                WriteObject(w, obj, playable);
            w.WriteEndArray();
            w.WriteEndObject();
        }

        buffer.WriteByte((byte)'\n');
        return buffer.ToArray();
    }

    // The Mode line of [General], read from the bytes; a missing line counts as 0, as lazer does.
    private static int ReadMode(byte[] bytes)
    {
        string text = Encoding.UTF8.GetString(bytes);
        bool inGeneral = false;
        foreach (string raw in text.Split('\n'))
        {
            string line = raw.Trim();
            if (line.StartsWith('['))
            {
                inGeneral = line == "[General]";
                continue;
            }

            if (inGeneral && line.StartsWith("Mode", StringComparison.Ordinal))
            {
                int colon = line.IndexOf(':');
                if (colon > 0 && line[..colon].Trim() == "Mode" && int.TryParse(line[(colon + 1)..].Trim(), out int mode))
                    return mode;
            }
        }

        return 0;
    }

    private static void WriteObject(Utf8JsonWriter w, HitObject obj, IBeatmap beatmap)
    {
        w.WriteStartObject();
        w.WriteString("type", obj.GetType().Name);
        WriteDouble(w, "start_time", obj.StartTime);
        if (obj is IHasDuration duration)
            WriteDouble(w, "end_time", duration.EndTime);

        if (obj is Slider slider)
        {
            w.WritePropertyName("slider");
            w.WriteStartObject();
            WriteDouble(w, "beat_length", beatmap.ControlPointInfo.TimingPointAt(slider.StartTime).BeatLength);
            WriteDouble(w, "slider_velocity_multiplier", slider.SliderVelocityMultiplier);
            WriteDouble(w, "velocity", slider.Velocity);
            WriteDouble(w, "path_distance", slider.Path.Distance);
            WriteDouble(w, "span_duration", slider.SpanDuration);
            WriteDouble(w, "tick_distance", slider.TickDistance);
            WriteDouble(w, "tick_distance_multiplier", slider.TickDistanceMultiplier);
            w.WriteBoolean("generate_ticks", slider.GenerateTicks);
            w.WriteNumber("repeat_count", slider.RepeatCount);
            w.WriteEndObject();
        }

        WriteSamples(w, "samples", obj.Samples);
        WriteSamples(w, "auxiliary_samples", obj.AuxiliarySamples);
        if (obj is Slider)
            WriteSamples(w, "sliding_samples", obj.CreateSlidingSamples());

        w.WritePropertyName("nested");
        w.WriteStartArray();
        foreach (var nested in obj.NestedHitObjects)
            WriteObject(w, nested, beatmap);
        w.WriteEndArray();
        w.WriteEndObject();
    }

    private static void WriteSamples(Utf8JsonWriter w, string property, IEnumerable<HitSampleInfo> samples)
    {
        w.WritePropertyName(property);
        w.WriteStartArray();
        foreach (var s in samples)
        {
            w.WriteStartObject();
            w.WriteString("kind", s switch
            {
                ConvertHitObjectParser.FileHitSampleInfo => "file",
                ConvertHitObjectParser.LegacyHitSampleInfo => "legacy",
                _ => "plain",
            });
            w.WriteString("name", s.Name);
            w.WriteString("bank", s.Bank);
            if (s.Suffix is null)
                w.WriteNull("suffix");
            else
                w.WriteString("suffix", s.Suffix);
            w.WriteNumber("volume", s.Volume);
            w.WriteBoolean("editor_auto_bank", s.EditorAutoBank);
            w.WriteBoolean("use_beatmap_samples", s.UseBeatmapSamples);
            if (s is ConvertHitObjectParser.LegacyHitSampleInfo legacy)
            {
                w.WriteNumber("custom_sample_bank", legacy.CustomSampleBank);
                w.WriteBoolean("is_layered", legacy.IsLayered);
                w.WriteBoolean("bank_specified", legacy.BankSpecified);
            }

            if (s is ConvertHitObjectParser.FileHitSampleInfo file)
                w.WriteString("filename", file.Filename);

            w.WritePropertyName("lookup_names");
            w.WriteStartArray();
            foreach (string lookup in s.LookupNames)
                w.WriteStringValue(lookup);
            w.WriteEndArray();
            w.WriteEndObject();
        }

        w.WriteEndArray();
    }

    // JSON has no NaN or infinity; lazer can produce them (a NaN beat length), so they are written as strings.
    private static void WriteDouble(Utf8JsonWriter w, string property, double value)
    {
        if (double.IsFinite(value))
            w.WriteNumber(property, value);
        else
            w.WriteString(property, value.ToString(System.Globalization.CultureInfo.InvariantCulture));
    }
}
