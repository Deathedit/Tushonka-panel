using System.Security.Cryptography;
using System.Text;
using SPTarkov.DI.Annotations;
using SPTarkov.Server.Core.Models.Common;
using SPTarkov.Server.Core.Models.Spt.Tables;
using SPTarkov.Server.Core.Utils;

namespace TushonkaPanelBridge;

public enum Change
{
    None,
    Added,
    Modified,
}

/// <summary>
/// Detects records that mods added or changed, in two steps:
/// <list type="bullet">
/// <item>Ids of items, traders and quests right after the database import, before any mod runs.
/// Anything not in this set was added later (the server requires mods to add items during Preload).</item>
/// <item>Content hashes after the server's own startup (PostLoad). A differing hash means a mod
/// edited the record afterwards, which is where mods usually tweak values.</item>
/// </list>
/// Edits made by mods during Preload are not detected. Trader assorts are excluded: they restock at runtime.
/// </summary>
[Injectable(InjectionType.Singleton)]
public sealed class Fingerprints(TemplateTable templates, TradersTable traders, JsonUtil json)
{
    private HashSet<MongoId>? _itemIds;
    private HashSet<MongoId>? _traderIds;
    private HashSet<MongoId>? _questIds;
    private Dictionary<MongoId, ulong>? _items;
    private Dictionary<MongoId, ulong>? _traders;
    private Dictionary<MongoId, ulong>? _quests;

    private readonly object _lock = new();
    private DateTime _cachedAt = DateTime.MinValue;
    private Dictionary<MongoId, Change> _itemChanges = [];
    private Dictionary<MongoId, Change> _traderChanges = [];
    private Dictionary<MongoId, Change> _questChanges = [];

    public bool HasBaseline => _items is not null && _itemIds is not null;

    public void TakeIdBaseline()
    {
        _itemIds = [.. templates.Items.Keys];
        _traderIds = [.. traders.Keys];
        _questIds = [.. templates.Quests.Keys];
    }

    public void TakeBaseline()
    {
        _items = HashAll(templates.Items);
        _traders = traders.ToDictionary(t => t.Key, t => Hash(t.Value.Base));
        _quests = HashAll(templates.Quests);
    }

    public Change Item(MongoId id) => Current().items.GetValueOrDefault(id);
    public Change Trader(MongoId id) => Current().traders.GetValueOrDefault(id);
    public Change Quest(MongoId id) => Current().quests.GetValueOrDefault(id);

    /// <summary>Change maps, recomputed at most every 30 seconds (hashing all items takes a moment).</summary>
    private (Dictionary<MongoId, Change> items, Dictionary<MongoId, Change> traders, Dictionary<MongoId, Change> quests) Current()
    {
        lock (_lock)
        {
            if (_items is null || _traders is null || _quests is null || _itemIds is null || _traderIds is null || _questIds is null)
            {
                return ([], [], []);
            }

            if (DateTime.UtcNow - _cachedAt > TimeSpan.FromSeconds(30))
            {
                _itemChanges = Diff(_itemIds, _items, HashAll(templates.Items));
                _traderChanges = Diff(_traderIds, _traders, traders.ToDictionary(t => t.Key, t => Hash(t.Value.Base)));
                _questChanges = Diff(_questIds, _quests, HashAll(templates.Quests));
                _cachedAt = DateTime.UtcNow;
            }

            return (_itemChanges, _traderChanges, _questChanges);
        }
    }

    private static Dictionary<MongoId, Change> Diff(HashSet<MongoId> stockIds, Dictionary<MongoId, ulong> baseline, Dictionary<MongoId, ulong> now)
    {
        var result = new Dictionary<MongoId, Change>();
        foreach (var (id, hash) in now)
        {
            if (!stockIds.Contains(id))
            {
                result[id] = Change.Added;
            }
            else if (baseline.TryGetValue(id, out var old) && old != hash)
            {
                result[id] = Change.Modified;
            }
        }

        return result;
    }

    private Dictionary<MongoId, ulong> HashAll<T>(Dictionary<MongoId, T> table)
    {
        var result = new Dictionary<MongoId, ulong>(table.Count);
        foreach (var (id, value) in table)
        {
            result[id] = Hash(value);
        }

        return result;
    }

    private ulong Hash<T>(T value)
    {
        var text = json.Serialize(value) ?? string.Empty;
        Span<byte> digest = stackalloc byte[16];
        MD5.HashData(Encoding.UTF8.GetBytes(text), digest);
        return BitConverter.ToUInt64(digest);
    }
}
