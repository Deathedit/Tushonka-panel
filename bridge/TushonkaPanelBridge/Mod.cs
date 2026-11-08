using SPTarkov.Common.Models.Logging;
using SPTarkov.DI.Annotations;
using SPTarkov.Server.Core.DI;
using SPTarkov.Server.Core.Models.Spt.Mod;
using Range = SemanticVersioning.Range;
using Version = SemanticVersioning.Version;

namespace TushonkaPanelBridge;

/// <summary>Mod metadata, discovered by the server's mod loader.</summary>
public sealed class BridgeMetadata : IModMetadata
{
    public string ModGuid { get; init; } = "com.sp-tushonka.panel-bridge";
    public string Name { get; init; } = "TushonkaPanelBridge";
    public string Author { get; init; } = "Tushonka Panel";
    public List<string>? Contributors { get; init; }
    public Version Version { get; init; } = new("0.1.0");
    public Range SptVersion { get; init; } = new("~4.1.0");
    public bool HasPrepatcher { get; init; } = false;
    public List<string>? Incompatibilities { get; init; }
    public Dictionary<string, Range>? ModDependencies { get; init; }
    public string? Url { get; init; } = "https://github.com/SP-Tushonka";
    public string License { get; init; } = "MIT";
}

/// <summary>Runs first, right after the database import: records which records are stock.</summary>
[Injectable(TypePriority = OnLoadOrder.Watermark + 1)]
public sealed class BridgeEarlyLoader(Fingerprints fingerprints) : IOnLoad
{
    public Task OnLoadAsync(CancellationToken cancellationToken)
    {
        if (!string.IsNullOrWhiteSpace(Environment.GetEnvironmentVariable("TUSHONKA_BRIDGE_TOKEN")))
        {
            fingerprints.TakeIdBaseline();
        }

        return Task.CompletedTask;
    }
}

/// <summary>
/// Runs at <see cref="OnLoadOrder.PostLoad"/>: after the server's own startup, before mods that use
/// PostLoad + 1 (the usual slot for database edits). Fingerprints the database here so the panel can
/// show which records mods added or changed, then starts the localhost API.
/// </summary>
[Injectable(TypePriority = OnLoadOrder.PostLoad)]
public sealed class BridgeLoader(ISptLogger<BridgeLoader> logger, Fingerprints fingerprints, BridgeServer server) : IOnLoad
{
    public Task OnLoadAsync(CancellationToken cancellationToken)
    {
        var token = Environment.GetEnvironmentVariable("TUSHONKA_BRIDGE_TOKEN");
        if (string.IsNullOrWhiteSpace(token) || token.Length < 32)
        {
            logger.Info("TushonkaPanelBridge: not started by Tushonka Panel (no token), staying idle.");
            return Task.CompletedTask;
        }

        var port = int.TryParse(Environment.GetEnvironmentVariable("TUSHONKA_BRIDGE_PORT"), out var p) && p is > 0 and < 65536 ? p : 6970;

        try
        {
            fingerprints.TakeBaseline();
        }
        catch (Exception e)
        {
            logger.Warning($"TushonkaPanelBridge: could not fingerprint the database, mod-change markers disabled: {e.Message}");
        }

        server.Start(port, token);
        return Task.CompletedTask;
    }
}
