using SPTarkov.DI.Annotations;
using SPTarkov.Server.Core.Models.Common;
using SPTarkov.Server.Core.Models.Eft.Common.Tables;
using SPTarkov.Server.Core.Models.Spt.Tables;
using SPTarkov.Server.Core.Services.Locales;
using SPTarkov.Server.Core.Utils;

namespace TushonkaPanelBridge;

public sealed record Page<T>(int Total, int Offset, List<T> Rows);

public sealed record Category(string Id, string Name, int Count);

public sealed record ItemRow(string Id, string Name, string ShortName, string Type, string CategoryId, string Category, string Size, double? Price, bool QuestItem, Change Changed);

public sealed record ItemPage(int Total, int Offset, List<ItemRow> Rows, List<Category> Categories, bool Fingerprints);

public sealed record ItemDetail(
    string Id, string Name, string ShortName, string Description, string Type, string Parent, string Category,
    string Size, double? Weight, int? StackMax, bool? CanSellOnRagfair, double? Price, bool QuestItem, Change Changed, RawJson Properties);

public sealed record TraderRow(string Id, string Name, string Currency, int LoyaltyLevels, int AssortItems, bool Insurance, bool Repair, Change Changed);

public sealed record LoyaltyLevelInfo(int Level, int? MinLevel, double? MinSalesSum, double? MinStanding);

public sealed record TraderDetail(string Id, string Name, string Currency, string Location, int AssortItems, bool Insurance, bool Repair, Change Changed, List<LoyaltyLevelInfo> LoyaltyLevels);

public sealed record QuestRow(string Id, string Name, string TraderId, string Trader, string Location, string Type, string Side, Change Changed);

public sealed record QuestPage(int Total, int Offset, List<QuestRow> Rows, List<Category> Traders, bool Fingerprints);

[Injectable(InjectionType.Singleton)]
public sealed class DatabaseApi(TemplateTable templates, TradersTable traders, LocaleService locales, JsonUtil json, Fingerprints fingerprints)
{
    public bool HasBaseline => fingerprints.HasBaseline;

    private static string Loc(Dictionary<string, string> locale, string key, string? fallback) =>
        locale.TryGetValue(key, out var v) && !string.IsNullOrWhiteSpace(v) ? v : fallback ?? key;

    private static bool Matches(string query, params string?[] fields) =>
        fields.Any(f => f is not null && f.Contains(query, StringComparison.OrdinalIgnoreCase));

    private static int Clamp(int limit) => Math.Clamp(limit, 1, 500);

    private static MongoId ParseId(string id) =>
        MongoId.IsValidMongoId(id) ? new MongoId(id) : throw new HttpError(400, "invalid id");

    // ---------- items ----------

    private List<ItemRow> AllItems()
    {
        var locale = locales.GetLocaleDb();
        var handbook = templates.Handbook;
        var handbookItems = (handbook.Items ?? []).GroupBy(i => i.Id).ToDictionary(g => g.Key, g => g.First());
        var categoryNames = (handbook.Categories ?? []).ToDictionary(c => c.Id.ToString(), c => Loc(locale, c.Id.ToString(), c.Id.ToString()));

        return templates.Items.Values
            .Where(i => !string.Equals(i.Type, "Node", StringComparison.OrdinalIgnoreCase))
            .Select(item =>
            {
                var id = item.Id.ToString();
                handbookItems.TryGetValue(item.Id, out var hb);
                var catId = hb?.ParentId.ToString() ?? "";
                return new ItemRow(
                    id,
                    Loc(locale, $"{id} Name", item.Properties?.Name ?? item.Name),
                    Loc(locale, $"{id} ShortName", item.Properties?.ShortName ?? ""),
                    item.Type ?? "Item",
                    catId,
                    catId == "" ? "Uncategorized" : categoryNames.GetValueOrDefault(catId, catId),
                    Size(item),
                    hb?.Price,
                    item.Properties?.QuestItem == true,
                    fingerprints.Item(item.Id)
                );
            })
            .OrderBy(r => r.Name.StartsWith("!!!", StringComparison.Ordinal))
            .ThenBy(r => SortKey(r.Name), StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    /// <summary>Sort by the first letter or digit, so names like "#FireKlean" or "(off)Black" sort naturally.</summary>
    private static string SortKey(string name) => new(name.SkipWhile(c => !char.IsLetterOrDigit(c)).ToArray());

    private static string Size(TemplateItem item) =>
        item.Properties?.Width is { } w && item.Properties?.Height is { } h ? $"{w}×{h}" : "";

    public ItemPage Items(string? query, string? category, bool changedOnly, int offset, int limit)
    {
        var all = AllItems();
        var categories = all.GroupBy(r => (r.CategoryId, r.Category))
            .Select(g => new Category(g.Key.CategoryId, g.Key.Category, g.Count()))
            .OrderBy(c => c.Name, StringComparer.OrdinalIgnoreCase)
            .ToList();

        IEnumerable<ItemRow> rows = all;
        if (!string.IsNullOrWhiteSpace(query))
        {
            rows = rows.Where(r => Matches(query.Trim(), r.Name, r.ShortName, r.Id));
        }

        if (!string.IsNullOrEmpty(category))
        {
            rows = rows.Where(r => r.CategoryId == category);
        }

        if (changedOnly)
        {
            rows = rows.Where(r => r.Changed != Change.None);
        }

        var list = rows.ToList();
        offset = Math.Max(0, offset);
        return new ItemPage(list.Count, offset, list.Skip(offset).Take(Clamp(limit)).ToList(), categories, fingerprints.HasBaseline);
    }

    public ItemDetail Item(string id)
    {
        var mid = ParseId(id);
        if (!templates.Items.TryGetValue(mid, out var item))
        {
            throw new HttpError(404, "item not found");
        }

        var locale = locales.GetLocaleDb();
        var hb = templates.Handbook.Items?.FirstOrDefault(i => i.Id == mid);
        var catId = hb?.ParentId.ToString();
        var p = item.Properties;
        return new ItemDetail(
            id,
            Loc(locale, $"{id} Name", p?.Name ?? item.Name),
            Loc(locale, $"{id} ShortName", p?.ShortName ?? ""),
            Loc(locale, $"{id} Description", p?.Description ?? ""),
            item.Type ?? "Item",
            item.Parent.ToString(),
            catId is null ? "Uncategorized" : Loc(locale, catId, catId),
            Size(item),
            p?.Weight,
            p?.StackMaxSize,
            p?.CanSellOnRagfair,
            hb?.Price,
            p?.QuestItem == true,
            fingerprints.Item(mid),
            new RawJson(json.Serialize(p) ?? "null")
        );
    }

    // ---------- traders ----------

    private TraderRow ToRow(Trader t) => new(
        t.Base.Id.ToString(),
        TraderName(t),
        t.Base.Currency?.ToString() ?? "",
        t.Base.LoyaltyLevels?.Count ?? 0,
        t.Assort?.Items?.Count ?? 0,
        t.Base.Insurance?.Availability == true,
        t.Base.Repair?.Availability == true,
        fingerprints.Trader(t.Base.Id)
    );

    public static string TraderName(Trader t) =>
        !string.IsNullOrWhiteSpace(t.Base.Nickname) ? t.Base.Nickname : !string.IsNullOrWhiteSpace(t.Base.Name) ? t.Base.Name : t.Base.Id.ToString();

    public List<TraderRow> Traders() =>
        traders.Values.Select(ToRow).OrderBy(r => r.Name, StringComparer.OrdinalIgnoreCase).ToList();

    public TraderDetail Trader(string id)
    {
        var mid = ParseId(id);
        if (!traders.TryGetValue(mid, out var t))
        {
            throw new HttpError(404, "trader not found");
        }

        var row = ToRow(t);
        var levels = (t.Base.LoyaltyLevels ?? [])
            .Select((l, i) => new LoyaltyLevelInfo(i + 1, l.MinLevel, l.MinSalesSum, l.MinStanding))
            .ToList();
        return new TraderDetail(row.Id, row.Name, row.Currency, t.Base.Location ?? "", row.AssortItems, row.Insurance, row.Repair, row.Changed, levels);
    }

    // ---------- quests ----------

    private List<QuestRow> AllQuests()
    {
        var locale = locales.GetLocaleDb();
        return templates.Quests.Values
            .Select(q =>
            {
                var id = q.Id.ToString();
                var trader = traders.TryGetValue(q.TraderId, out var t) ? TraderName(t) : q.TraderId.ToString();
                var location = q.Location is null or "any" ? "Any" : Loc(locale, $"{q.Location} Name", q.Location);
                return new QuestRow(id, QuestName(q, locale), q.TraderId.ToString(), trader, location, q.Type.ToString(), q.Side ?? "", fingerprints.Quest(q.Id));
            })
            .OrderBy(r => r.Name, StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    public static string QuestName(Quest q, Dictionary<string, string> locale)
    {
        var id = q.Id.ToString();
        if (locale.TryGetValue($"{id} name", out var n) && !string.IsNullOrWhiteSpace(n))
        {
            return n;
        }

        return !string.IsNullOrWhiteSpace(q.QuestName) ? q.QuestName : !string.IsNullOrWhiteSpace(q.Name) ? q.Name : id;
    }

    public QuestPage Quests(string? query, string? trader, bool changedOnly, int offset, int limit)
    {
        var all = AllQuests();
        var traderList = all.GroupBy(r => (r.TraderId, r.Trader))
            .Select(g => new Category(g.Key.TraderId, g.Key.Trader, g.Count()))
            .OrderBy(c => c.Name, StringComparer.OrdinalIgnoreCase)
            .ToList();

        IEnumerable<QuestRow> rows = all;
        if (!string.IsNullOrWhiteSpace(query))
        {
            rows = rows.Where(r => Matches(query.Trim(), r.Name, r.Id, r.Location));
        }

        if (!string.IsNullOrEmpty(trader))
        {
            rows = rows.Where(r => r.TraderId == trader);
        }

        if (changedOnly)
        {
            rows = rows.Where(r => r.Changed != Change.None);
        }

        var list = rows.ToList();
        offset = Math.Max(0, offset);
        return new QuestPage(list.Count, offset, list.Skip(offset).Take(Clamp(limit)).ToList(), traderList, fingerprints.HasBaseline);
    }

    public object Quest(string id)
    {
        var mid = ParseId(id);
        if (!templates.Quests.TryGetValue(mid, out var q))
        {
            throw new HttpError(404, "quest not found");
        }

        var row = AllQuests().First(r => r.Id == id);
        var locale = locales.GetLocaleDb();
        return new
        {
            row.Id, row.Name, row.TraderId, row.Trader, row.Location, row.Type, row.Side, row.Changed,
            Description = Loc(locale, $"{id} description", ""),
            q.Restartable,
            Raw = new RawJson(json.Serialize(q) ?? "null"),
        };
    }
}
