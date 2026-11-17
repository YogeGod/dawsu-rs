// The lazer oracle (ADRs 0026, 0032): writes lazer's expected results for .osu files as JSON.
// Usage: LazerOracle <output folder> <input folder>...
//        LazerOracle --audio <output folder> <input folder>...
// Every .osu file under each input folder gets <output folder>/<input folder name>/<relative path>.json.
// With --audio, every .mp3, .ogg and .wav file gets the same, describing where lazer's decode of it starts.
// Output is written to a temporary folder and moved into place only when every file succeeded.

using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using ManagedBass;
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
    private const int ResultFormatVersion = 2;

    public static int Main(string[] args)
    {
        bool audio = args.Length > 0 && args[0] == "--audio";
        if (audio)
            args = args.Skip(1).ToArray();

        if (args.Length < 2)
        {
            Console.Error.WriteLine("usage: LazerOracle [--audio] <output folder> <input folder>...");
            return 2;
        }

        if (audio)
            InitBassAsLazerDoes();

        string output = Path.GetFullPath(args[0]);
        string staging = output + ".staging-" + Guid.NewGuid().ToString("N");
        string lazerVersion = typeof(Beatmap).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion ?? "unknown";

        try
        {
            int count = 0;
            foreach (string input in args.Skip(1).Select(Path.GetFullPath))
            {
                string group = Path.GetFileName(input.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar));
                var files = Directory.EnumerateFiles(input, "*", SearchOption.AllDirectories)
                                     .Where(f => audio ? AudioExtensions.Contains(Path.GetExtension(f).ToLowerInvariant()) : f.EndsWith(".osu", StringComparison.Ordinal))
                                     .OrderBy(f => f, StringComparer.Ordinal);

                foreach (string file in files)
                {
                    string relative = Path.GetRelativePath(input, file).Replace('\\', '/');
                    string target = Path.Combine(staging, group, relative + ".json");
                    Directory.CreateDirectory(Path.GetDirectoryName(target)!);
                    byte[] bytes = File.ReadAllBytes(file);
                    File.WriteAllBytes(target, audio ? DescribeAudio(file, bytes, group + "/" + relative, lazerVersion) : Describe(bytes, group + "/" + relative, lazerVersion));
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

    private static readonly string[] AudioExtensions = [".mp3", ".ogg", ".wav"];

    // The thresholds the first non-silent frame is reported at; 0 means the first frame with any sample not exactly zero.
    private static readonly float[] Thresholds = [0f, 1e-4f, 1e-3f, 1e-2f];

    // osu!framework's AudioManager.InitBass sets option 68 (BASS_CONFIG_MP3_OLDGAPS) to 1 before BASS starts
    // (osu-framework 06e28ec, osu.Framework/Audio/AudioManager.cs:430-434). The oracle does the same, then
    // starts BASS on the no-sound device, since it only decodes.
    private static void InitBassAsLazerDoes()
    {
        Bass.Configure((Configuration)68, 1);
        if (!Bass.Init(0))
            throw new InvalidOperationException($"BASS did not start: {Bass.LastError}");
    }

    // Where lazer's decode of a song starts: lazer's tracks are decode streams with BassFlags.Decode | BassFlags.Prescan
    // (TrackBass.cs:159). Samples are read as floats here so the thresholds compare with dawsu!'s decoder;
    // the sample format does not move the start.
    private static byte[] DescribeAudio(string path, byte[] bytes, string name, string lazerVersion)
    {
        int stream = Bass.CreateStream(path, 0, 0, BassFlags.Decode | BassFlags.Prescan | BassFlags.Float);
        if (stream == 0)
            throw new InvalidOperationException($"{name}: BASS could not open it: {Bass.LastError}");

        try
        {
            Bass.ChannelGetInfo(stream, out ChannelInfo info);
            int channels = info.Channels;
            long frames = 0;
            long?[] first = new long?[Thresholds.Length];
            float[] chunk = new float[channels * 4096];

            while (true)
            {
                int got = Bass.ChannelGetData(stream, chunk, chunk.Length * sizeof(float) | (int)DataFlags.Float);
                if (got <= 0)
                    break;

                int samples = got / sizeof(float);
                for (int i = 0; i < samples; i++)
                {
                    float a = Math.Abs(chunk[i]);
                    for (int t = 0; t < Thresholds.Length; t++)
                    {
                        if (first[t] == null && a > Thresholds[t])
                            first[t] = frames + i / channels;
                    }
                }

                frames += samples / channels;
            }

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
                w.WriteString("bass_version", Bass.Version.ToString());
                w.WriteNumber("mp3_oldgaps", Bass.GetConfig((Configuration)68));
                w.WriteEndObject();

                w.WriteNumber("sample_rate", info.Frequency);
                w.WriteNumber("channels", channels);
                w.WriteNumber("decoded_frames", frames);
                w.WritePropertyName("first_frame_above");
                w.WriteStartArray();
                for (int t = 0; t < Thresholds.Length; t++)
                {
                    w.WriteStartObject();
                    w.WriteNumber("threshold", Thresholds[t]);
                    if (first[t] is long f)
                        w.WriteNumber("frame", f);
                    else
                        w.WriteNull("frame");
                    w.WriteEndObject();
                }

                w.WriteEndArray();
                w.WriteEndObject();
            }

            buffer.WriteByte((byte)'\n');
            return buffer.ToArray();
        }
        finally
        {
            Bass.StreamFree(stream);
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
        if (obj is Slider sliderWithTail)
        {
            WriteSamples(w, "sliding_samples", obj.CreateSlidingSamples());
            // The slider plays these at its end itself; its tail circle carries none (Slider.UpdateNestedSamples).
            WriteSamples(w, "tail_samples", sliderWithTail.TailSamples);
        }

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
