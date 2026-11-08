using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using SPTarkov.Common.Models.Logging;
using SPTarkov.DI.Annotations;

namespace TushonkaPanelBridge;

/// <summary>Thrown by handlers to return a specific status code with a message.</summary>
public sealed class HttpError(int status, string message) : Exception(message)
{
    public int Status { get; } = status;
}

/// <summary>
/// Plain-HTTP JSON API on 127.0.0.1 only, for Tushonka Panel. Every request must carry the
/// per-run token the panel passes in through the environment.
/// </summary>
[Injectable(InjectionType.Singleton)]
public sealed class BridgeServer(ISptLogger<BridgeServer> logger, DatabaseApi db, ProfileApi profiles)
{
    public static readonly JsonSerializerOptions Json = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Converters = { new JsonStringEnumConverter(JsonNamingPolicy.CamelCase) },
    };

    private HttpListener? _listener;
    private byte[] _token = [];

    public void Start(int port, string token)
    {
        _token = Encoding.UTF8.GetBytes(token);
        var listener = new HttpListener();
        listener.Prefixes.Add($"http://127.0.0.1:{port}/");
        try
        {
            listener.Start();
        }
        catch (Exception e)
        {
            logger.Error($"TushonkaPanelBridge: could not listen on 127.0.0.1:{port}: {e.Message}");
            return;
        }

        _listener = listener;
        logger.Success($"TushonkaPanelBridge: listening on 127.0.0.1:{port}");
        _ = Task.Run(AcceptLoop);
    }

    private async Task AcceptLoop()
    {
        while (_listener is { IsListening: true } listener)
        {
            HttpListenerContext ctx;
            try
            {
                ctx = await listener.GetContextAsync();
            }
            catch
            {
                break;
            }

            _ = Task.Run(() => Handle(ctx));
        }
    }

    private async Task Handle(HttpListenerContext ctx)
    {
        var res = ctx.Response;
        try
        {
            if (!Authorized(ctx.Request))
            {
                throw new HttpError(401, "invalid token");
            }

            var result = await Route(ctx.Request);
            switch (result)
            {
                case RawJson raw:
                    await Write(res, 200, raw.Text);
                    break;
                default:
                    await Write(res, 200, JsonSerializer.Serialize(result, Json));
                    break;
            }
        }
        catch (HttpError e)
        {
            await Write(res, e.Status, JsonSerializer.Serialize(new { error = e.Message }, Json));
        }
        catch (Exception e)
        {
            logger.Error($"TushonkaPanelBridge: {ctx.Request.HttpMethod} {ctx.Request.Url?.AbsolutePath} failed: {e}");
            await Write(res, 500, JsonSerializer.Serialize(new { error = e.Message }, Json));
        }
    }

    private bool Authorized(HttpListenerRequest req)
    {
        var header = req.Headers["Authorization"];
        if (header is null || !header.StartsWith("Bearer ", StringComparison.Ordinal))
        {
            return false;
        }

        var given = Encoding.UTF8.GetBytes(header[7..]);
        return given.Length == _token.Length && CryptographicOperations.FixedTimeEquals(given, _token);
    }

    private async Task<object> Route(HttpListenerRequest req)
    {
        var parts = (req.Url?.AbsolutePath ?? "/").Trim('/').Split('/', StringSplitOptions.RemoveEmptyEntries)
            .Select(Uri.UnescapeDataString).ToArray();
        var q = req.QueryString;
        var method = req.HttpMethod;

        return (method, parts) switch
        {
            ("GET", ["health"]) => new
            {
                ok = true,
                bridgeVersion = typeof(BridgeServer).Assembly.GetName().Version?.ToString(3),
                fingerprints = db.HasBaseline,
            },
            ("GET", ["db", "items"]) => db.Items(q["q"], q["category"], q["changed"] == "1", Int(q["offset"], 0), Int(q["limit"], 100)),
            ("GET", ["db", "items", var id]) => db.Item(id),
            ("GET", ["db", "traders"]) => db.Traders(),
            ("GET", ["db", "traders", var id]) => db.Trader(id),
            ("GET", ["db", "quests"]) => db.Quests(q["q"], q["trader"], q["changed"] == "1", Int(q["offset"], 0), Int(q["limit"], 100)),
            ("GET", ["db", "quests", var id]) => db.Quest(id),
            ("GET", ["profiles"]) => profiles.List(),
            ("GET", ["editions"]) => profiles.Editions(),
            ("POST", ["profiles"]) => await profiles.Create(await ReadBody<ProfileCreate>(req)),
            ("GET", ["profiles", var id]) => profiles.Get(id),
            ("GET", ["profiles", var id, "raw"]) => new RawJson(profiles.Raw(id)),
            ("DELETE", ["profiles", var id]) => profiles.Delete(id),
            ("PATCH", ["profiles", var id]) => await profiles.Patch(id, await ReadBody<ProfilePatch>(req)),
            _ => throw new HttpError(404, "not found"),
        };
    }

    private static int Int(string? s, int fallback) => int.TryParse(s, out var v) ? v : fallback;

    private static async Task<T> ReadBody<T>(HttpListenerRequest req)
    {
        if (req.ContentLength64 > 4 * 1024 * 1024)
        {
            throw new HttpError(413, "body too large");
        }

        try
        {
            return await JsonSerializer.DeserializeAsync<T>(req.InputStream, Json) ?? throw new HttpError(400, "empty body");
        }
        catch (JsonException e)
        {
            throw new HttpError(400, $"invalid JSON: {e.Message}");
        }
    }

    private static async Task Write(HttpListenerResponse res, int status, string body)
    {
        try
        {
            var bytes = Encoding.UTF8.GetBytes(body);
            res.StatusCode = status;
            res.ContentType = "application/json; charset=utf-8";
            res.ContentLength64 = bytes.Length;
            await res.OutputStream.WriteAsync(bytes);
            res.Close();
        }
        catch
        {
            // client went away
        }
    }
}

/// <summary>Already-serialized JSON (from the server's own serializer) passed through unchanged.</summary>
[JsonConverter(typeof(RawJsonConverter))]
public sealed record RawJson(string Text);

public sealed class RawJsonConverter : JsonConverter<RawJson>
{
    public override RawJson Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) =>
        new(JsonDocument.ParseValue(ref reader).RootElement.GetRawText());

    public override void Write(Utf8JsonWriter writer, RawJson value, JsonSerializerOptions options) =>
        writer.WriteRawValue(value.Text, skipInputValidation: true);
}
