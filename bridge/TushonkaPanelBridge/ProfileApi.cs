using System.Text.RegularExpressions;
using SPTarkov.DI.Annotations;
using SPTarkov.Server.Core.Helpers.Profile;
using SPTarkov.Server.Core.Models.Common;
using SPTarkov.Server.Core.Models.Eft.Common;
using SPTarkov.Server.Core.Models.Eft.Common.Tables;
using SPTarkov.Server.Core.Models.Eft.Profile;
using SPTarkov.Server.Core.Models.Enums;
using SPTarkov.Server.Core.Models.Enums.Hideout;
using SPTarkov.Server.Core.Models.Spt.Tables;
using SPTarkov.Server.Core.Servers;
using SPTarkov.Server.Core.Services.Locales;
using SPTarkov.Server.Core.Services.Profile;
using SPTarkov.Server.Core.Utils;

namespace TushonkaPanelBridge;

public sealed record ProfileRow(string Id, string Username, string Nickname, string Side, string Edition, int? Level, bool HasCharacter, bool Online, bool Invalid);

public sealed record SkillState(string Id, string Label, int Level, double Progress);
public sealed record TraderState(string Id, string Label, int LoyaltyLevel, int MaxLoyaltyLevel, double Standing, double SalesSum, bool Unlocked);
public sealed record HideoutState(string Type, string Label, int Level, int MaxLevel, bool Constructing);
public sealed record QuestState(string Id, string Label, string Trader, string Status);
public sealed record QuestOption(string Id, string Label, string Trader);

public sealed record ProfileState(
    string Id, string Username, string Edition, bool HasCharacter, bool Online, bool Invalid,
    string Nickname, string Side, int? PmcLevel, int? PmcExperience, int? ScavLevel, int? ScavExperience, int MaxLevel, int PrestigeLevel,
    List<SkillState> Skills, List<TraderState> Traders, List<HideoutState> Hideout, List<QuestState> Quests, List<QuestOption> AvailableQuests);

public sealed record SkillPatch(string Id, int Level, double? Progress);
public sealed record TraderPatch(string Id, int? LoyaltyLevel, double? Standing, double? SalesSum, bool? Unlocked);
public sealed record HideoutPatch(string Type, int Level);
/// <param name="Action">"start" (adds the quest if missing, or restarts it) or "availableForFinish".</param>
public sealed record QuestPatch(string Id, string Action);

public sealed record ProfileCreate(string Username, string? Edition, string? Side, string? Nickname);

public sealed record ProfilePatch(
    string? Nickname, int? PmcLevel, int? ScavLevel,
    List<SkillPatch>? Skills, List<TraderPatch>? Traders, List<HideoutPatch>? Hideout, List<QuestPatch>? Quests);

/// <summary>
/// Profile reads and edits against the server's in-memory profiles. Edits follow the same rules as
/// the built-in web UI's profile editor and are saved with <see cref="SaveServer.SaveProfileAsync"/>,
/// so the server's next autosave keeps them.
/// </summary>
[Injectable(InjectionType.Singleton)]
public sealed partial class ProfileApi(
    SaveServer saveServer,
    ProfileHelper profileHelper,
    ProfileActivityService activity,
    TemplateTable templates,
    TradersTable traders,
    HideoutTable hideout,
    LocaleService locales,
    JsonUtil json,
    HashUtil hashUtil,
    CreateProfileService createProfileService)
{
    private static readonly SemaphoreSlim EditLock = new(1, 1);

    private SptProfileRef Load(string id)
    {
        if (!MongoId.IsValidMongoId(id))
        {
            throw new HttpError(400, "invalid profile id");
        }

        var mid = new MongoId(id);
        if (!saveServer.ProfileExists(mid))
        {
            throw new HttpError(404, "profile not found");
        }

        return new SptProfileRef(mid, saveServer.GetProfile(mid));
    }

    private sealed record SptProfileRef(MongoId Id, SPTarkov.Server.Core.Models.Eft.Profile.SptProfile Profile);

    private bool IsOnline(MongoId id) => activity.ActiveWithinLastMinutes(id, 3);

    public List<ProfileRow> List() =>
        saveServer.GetProfiles()
            .Select(kv =>
            {
                var info = kv.Value.ProfileInfo;
                var pmc = kv.Value.CharacterData?.PmcData?.Info;
                return new ProfileRow(
                    kv.Key.ToString(),
                    info?.Username ?? "",
                    pmc?.Nickname ?? "",
                    pmc?.Side ?? "",
                    info?.Edition ?? "",
                    pmc?.Level,
                    !string.IsNullOrWhiteSpace(pmc?.Nickname),
                    IsOnline(kv.Key),
                    info?.InvalidOrUnloadableProfile == true
                );
            })
            .OrderBy(r => r.Username, StringComparer.OrdinalIgnoreCase)
            .ToList();

    public List<string> Editions() => templates.Profiles.Keys.OrderBy(k => k).ToList();

    /// <summary>
    /// Creates an account and, when a nickname is given, its PMC character, the same way the
    /// launcher (register) and game client (character creation) would.
    /// </summary>
    public async Task<ProfileState> Create(ProfileCreate req)
    {
        var username = req.Username?.Trim() ?? "";
        if (username.Length is < 1 or > 32)
        {
            throw new HttpError(400, "username must be 1-32 characters");
        }

        if (saveServer.GetProfiles().Values.Any(p => string.Equals(p.ProfileInfo?.Username, username, StringComparison.OrdinalIgnoreCase)))
        {
            throw new HttpError(409, "a profile with that username already exists");
        }

        var edition = req.Edition ?? (templates.Profiles.ContainsKey("Standard") ? "Standard" : templates.Profiles.Keys.First());
        if (!templates.Profiles.TryGetValue(edition, out var sides))
        {
            throw new HttpError(400, $"unknown edition: {edition}");
        }

        var side = string.Equals(req.Side, "Bear", StringComparison.OrdinalIgnoreCase) ? "Bear" : "Usec";
        var nickname = req.Nickname?.Trim();
        if (!string.IsNullOrEmpty(nickname))
        {
            Validate(new ProfilePatch(nickname, null, null, null, null, null, null));
            if (saveServer.GetProfiles().Values.Any(p => string.Equals(p.CharacterData?.PmcData?.Info?.Nickname, nickname, StringComparison.OrdinalIgnoreCase)))
            {
                throw new HttpError(409, "that nickname is taken");
            }
        }

        var id = new MongoId();
        await EditLock.WaitAsync();
        try
        {
            saveServer.CreateProfile(new SPTarkov.Server.Core.Models.Eft.Profile.Info
            {
                ProfileId = id,
                ScavengerId = new MongoId(),
                Aid = hashUtil.GenerateAccountId(),
                Username = username,
                IsWiped = true,
                Edition = edition,
            });
            await saveServer.LoadProfileAsync(id);

            if (!string.IsNullOrEmpty(nickname))
            {
                try
                {
                    var template = (side == "Bear" ? sides.Bear : sides.Usec) ?? throw new HttpError(500, $"no {side} template for {edition}");
                    await createProfileService.CreateProfile(id, new ProfileCreateRequestData
                    {
                        Side = side,
                        Nickname = nickname,
                        HeadId = DefaultCustomization(template, side, CustomisationTypeId.HEAD),
                        VoiceId = DefaultCustomization(template, side, CustomisationTypeId.VOICE),
                    });
                }
                catch (Exception e) when (e is not HttpError)
                {
                    // CreateProfile removes the account before rebuilding it; don't leave half a profile behind.
                    saveServer.RemoveProfile(id);
                    throw new HttpError(500, $"character creation failed: {e.Message}");
                }
            }

            await saveServer.SaveProfileAsync(id);
        }
        finally
        {
            EditLock.Release();
        }

        return Get(id.ToString());
    }

    private MongoId DefaultCustomization(TemplateSide template, string side, MongoId parentId)
    {
        var value = parentId == CustomisationTypeId.HEAD ? template.Character?.Customization?.Head : template.Character?.Customization?.Voice;
        if (value is { IsEmpty: false } v)
        {
            return v;
        }

        return templates.Customization.Values
            .FirstOrDefault(c => c.Parent == parentId && (c.Properties?.Side?.Any(s => string.Equals(s, side, StringComparison.OrdinalIgnoreCase)) ?? false))
            ?.Id ?? throw new HttpError(500, $"no default customization for {side}");
    }

    /// <summary>Deletes a profile after writing a copy to user/profiles-deleted/.</summary>
    public object Delete(string id)
    {
        var (mid, profile) = Load(id);
        if (IsOnline(mid))
        {
            throw new HttpError(409, "the player is online; delete the profile when they are not playing");
        }

        var dir = System.IO.Path.Combine("user", "profiles-deleted");
        Directory.CreateDirectory(dir);
        var backup = System.IO.Path.Combine(dir, $"{mid}-{DateTime.Now:yyyyMMdd-HHmmss}.json");
        File.WriteAllText(backup, json.Serialize(profile, indented: true) ?? "null");
        if (!saveServer.RemoveProfile(mid))
        {
            throw new HttpError(500, "the profile file could not be deleted");
        }

        return new { ok = true, backup };
    }

    public string Raw(string id) => json.Serialize(Load(id).Profile, indented: true) ?? "null";

    public ProfileState Get(string id)
    {
        var (mid, profile) = Load(id);
        var info = profile.ProfileInfo;
        var pmc = profile.CharacterData?.PmcData;
        var scav = profile.CharacterData?.ScavData;
        var locale = locales.GetLocaleDb();
        var questIds = pmc?.Quests?.Select(q => q.QId).ToHashSet() ?? [];

        var skills = pmc?.Skills?.Common?
            .Select(s =>
            {
                var progress = s.Progress;
                var level = Math.Clamp((int)Math.Floor(progress / 100d), 0, 51);
                var inLevel = level >= 51 ? 0d : Math.Round(Math.Clamp(progress - level * 100d, 0d, 99.99d), 2);
                return new SkillState(s.Id.ToString(), FormatName(s.Id.ToString()), level, inLevel);
            })
            .OrderBy(s => s.Label)
            .ToList() ?? [];

        var traderStates = pmc?.TradersInfo?
            .Select(kv =>
            {
                var max = MaxLoyalty(kv.Key);
                return new TraderState(
                    kv.Key.ToString(),
                    traders.TryGetValue(kv.Key, out var t) ? DatabaseApi.TraderName(t) : kv.Key.ToString(),
                    Math.Clamp(kv.Value.LoyaltyLevel ?? 1, 1, max),
                    max,
                    Math.Round(kv.Value.Standing ?? 0d, 2),
                    Math.Max(0d, kv.Value.SalesSum ?? 0d),
                    kv.Value.Unlocked == true
                );
            })
            .OrderBy(t => t.Label)
            .ToList() ?? [];

        var areas = pmc?.Hideout?.Areas?
            .Select(a => new HideoutState(a.Type.ToString(), FormatName(a.Type.ToString()), Math.Max(0, a.Level ?? 0), MaxAreaLevel(a.Type, a.Level ?? 0), a.Constructing == true))
            .OrderBy(a => a.Label)
            .ToList() ?? [];

        var quests = pmc?.Quests?
            .Select(q =>
            {
                templates.Quests.TryGetValue(q.QId, out var dbq);
                return new QuestState(q.QId.ToString(), dbq is null ? q.QId.ToString() : DatabaseApi.QuestName(dbq, locale), dbq is null ? "" : TraderLabel(dbq.TraderId), q.Status.ToString());
            })
            .OrderBy(q => q.Label)
            .ToList() ?? [];

        var available = templates.Quests.Values
            .Where(q => !questIds.Contains(q.Id))
            .Select(q => new QuestOption(q.Id.ToString(), DatabaseApi.QuestName(q, locale), TraderLabel(q.TraderId)))
            .OrderBy(q => q.Label)
            .ToList();

        return new ProfileState(
            mid.ToString(),
            info?.Username ?? "",
            info?.Edition ?? "",
            !string.IsNullOrWhiteSpace(pmc?.Info?.Nickname),
            IsOnline(mid),
            info?.InvalidOrUnloadableProfile == true,
            pmc?.Info?.Nickname ?? "",
            pmc?.Info?.Side ?? "",
            pmc?.Info?.Level,
            pmc?.Info?.Experience,
            scav?.Info?.Level,
            scav?.Info?.Experience,
            profileHelper.GetMaxLevel(),
            pmc?.Info?.PrestigeLevel ?? 0,
            skills,
            traderStates,
            areas,
            quests,
            available
        );
    }

    public async Task<ProfileState> Patch(string id, ProfilePatch patch)
    {
        var (mid, profile) = Load(id);
        var pmc = profile.CharacterData?.PmcData;
        if (pmc?.Info is null || string.IsNullOrWhiteSpace(pmc.Info.Nickname))
        {
            throw new HttpError(409, "this profile has no character yet");
        }

        Validate(patch);

        await EditLock.WaitAsync();
        try
        {
            if (patch.Nickname is { } nick)
            {
                pmc.Info.Nickname = nick.Trim();
                pmc.Info.LowerNickname = pmc.Info.Nickname.ToLowerInvariant();
            }

            if (patch.PmcLevel is { } pmcLevel)
            {
                pmc.Info.Level = pmcLevel;
                pmc.Info.Experience = ExperienceFor(pmcLevel, pmc.Info.Experience);
            }

            var scavInfo = profile.CharacterData?.ScavData?.Info;
            if (patch.ScavLevel is { } scavLevel && scavInfo is not null)
            {
                scavInfo.Level = scavLevel;
                scavInfo.Experience = ExperienceFor(scavLevel, scavInfo.Experience);
            }

            ApplySkills(pmc, patch.Skills);
            ApplyTraders(pmc, patch.Traders);
            ApplyHideout(pmc, patch.Hideout);
            ApplyQuests(pmc, patch.Quests);

            await saveServer.SaveProfileAsync(mid);
        }
        finally
        {
            EditLock.Release();
        }

        return Get(id);
    }

    private void Validate(ProfilePatch patch)
    {
        var maxLevel = profileHelper.GetMaxLevel();
        if (patch.Nickname is { } n && (n.Trim().Length is < 3 or > 15 || !NicknamePattern().IsMatch(n.Trim())))
        {
            throw new HttpError(400, "nickname must be 3-15 letters, digits, '-' or '_'");
        }

        if (patch.PmcLevel is < 1 || patch.PmcLevel > maxLevel || patch.ScavLevel is < 1 || patch.ScavLevel > maxLevel)
        {
            throw new HttpError(400, $"level must be between 1 and {maxLevel}");
        }

        if (patch.Skills?.Any(s => s.Level is < 0 or > 51 || s.Progress is < 0 or >= 100) == true)
        {
            throw new HttpError(400, "skill level must be 0-51 and progress 0-99.99");
        }

        if (patch.Traders?.Any(t => t.SalesSum is < 0 || t.LoyaltyLevel is < 1) == true)
        {
            throw new HttpError(400, "trader values out of range");
        }

        if (patch.Hideout?.Any(h => h.Level < 0) == true)
        {
            throw new HttpError(400, "hideout level cannot be negative");
        }

        if (patch.Quests?.Any(q => q.Action is not ("start" or "availableForFinish")) == true)
        {
            throw new HttpError(400, "quest action must be 'start' or 'availableForFinish'");
        }
    }

    [GeneratedRegex("^[A-Za-z0-9_-]+$")]
    private static partial Regex NicknamePattern();

    /// <summary>Keeps the current experience if it already matches the level, otherwise uses the level's minimum.</summary>
    private int ExperienceFor(int level, int? current)
    {
        if (current is { } xp && profileHelper.GetLevelFromExperience(xp) == level)
        {
            return xp;
        }

        return profileHelper.GetExperience(level) ?? 0;
    }

    private static void ApplySkills(PmcData pmc, List<SkillPatch>? edits)
    {
        var common = pmc.Skills?.Common;
        if (edits is null || common is null)
        {
            return;
        }

        foreach (var edit in edits)
        {
            var skill = common.FirstOrDefault(s => string.Equals(s.Id.ToString(), edit.Id, StringComparison.Ordinal));
            if (skill is null)
            {
                continue;
            }

            skill.Progress = edit.Level >= 51 ? 5100d : edit.Level * 100d + Math.Clamp(edit.Progress ?? 0d, 0d, 99.99d);
        }
    }

    private void ApplyTraders(PmcData pmc, List<TraderPatch>? edits)
    {
        if (edits is null || pmc.TradersInfo is null)
        {
            return;
        }

        foreach (var edit in edits)
        {
            if (!MongoId.IsValidMongoId(edit.Id) || !pmc.TradersInfo.TryGetValue(new MongoId(edit.Id), out var info))
            {
                continue;
            }

            if (edit.LoyaltyLevel is { } ll)
            {
                info.LoyaltyLevel = Math.Clamp(ll, 1, MaxLoyalty(new MongoId(edit.Id)));
            }

            if (edit.Standing is { } standing)
            {
                info.Standing = Math.Round(standing, 2);
            }

            if (edit.SalesSum is { } sales)
            {
                info.SalesSum = Math.Max(0d, sales);
            }

            if (edit.Unlocked is { } unlocked)
            {
                info.Unlocked = unlocked;
            }
        }
    }

    private void ApplyHideout(PmcData pmc, List<HideoutPatch>? edits)
    {
        var areas = pmc.Hideout?.Areas;
        if (edits is null || areas is null)
        {
            return;
        }

        foreach (var edit in edits)
        {
            if (!Enum.TryParse<HideoutAreas>(edit.Type, out var type))
            {
                continue;
            }

            var area = areas.FirstOrDefault(a => a.Type == type);
            if (area is null)
            {
                continue;
            }

            area.Level = Math.Clamp(edit.Level, 0, MaxAreaLevel(type, area.Level ?? 0));
            area.Constructing = false;
            area.CompleteTime = 0;
        }
    }

    private void ApplyQuests(PmcData pmc, List<QuestPatch>? edits)
    {
        if (edits is null)
        {
            return;
        }

        pmc.Quests ??= [];
        var now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        foreach (var edit in edits)
        {
            if (!MongoId.IsValidMongoId(edit.Id))
            {
                continue;
            }

            var qid = new MongoId(edit.Id);
            var quest = pmc.Quests.FirstOrDefault(q => q.QId == qid);
            if (quest is null)
            {
                if (edit.Action != "start" || !templates.Quests.ContainsKey(qid))
                {
                    continue;
                }

                pmc.Quests.Add(new()
                {
                    QId = qid,
                    StartTime = now,
                    Status = QuestStatusEnum.Started,
                    StatusTimers = new Dictionary<QuestStatusEnum, double> { [QuestStatusEnum.Started] = now },
                    CompletedConditions = [],
                });
                continue;
            }

            quest.StatusTimers ??= [];
            if (edit.Action == "availableForFinish")
            {
                quest.Status = QuestStatusEnum.AvailableForFinish;
                quest.AvailableAfter = null;
                quest.StatusTimers[QuestStatusEnum.AvailableForFinish] = now;
            }
            else
            {
                quest.Status = QuestStatusEnum.Started;
                quest.StartTime = now;
                quest.AvailableAfter = null;
                quest.StatusTimers[QuestStatusEnum.Started] = now;
                foreach (var later in quest.StatusTimers.Keys.Where(s => s > QuestStatusEnum.Started).ToList())
                {
                    quest.StatusTimers.Remove(later);
                }

                quest.CompletedConditions = [];
            }
        }
    }

    private int MaxLoyalty(MongoId traderId) =>
        Math.Max(1, traders.TryGetValue(traderId, out var t) ? t.Base.LoyaltyLevels?.Count ?? 1 : 1);

    private string TraderLabel(MongoId traderId) =>
        traders.TryGetValue(traderId, out var t) ? DatabaseApi.TraderName(t) : traderId.ToString();

    private int MaxAreaLevel(HideoutAreas type, int current)
    {
        var area = hideout.Areas.Concat(hideout.CustomAreas ?? []).FirstOrDefault(a => a.Type == type);
        if (area?.Stages is null || area.Stages.Count == 0)
        {
            return Math.Max(current, 1);
        }

        var max = area.Stages.Keys.Select(k => int.TryParse(k, out var l) ? l : 0).DefaultIfEmpty(0).Max();
        return Math.Max(current, max);
    }

    private static string FormatName(string value) => value switch
    {
        "DMR" or "HMG" or "LMG" or "SMG" => value,
        "MagDrills" => "Magazine Drills",
        "WeaponTreatment" => "Weapon Maintenance",
        "TroubleShooting" => "Troubleshooting",
        _ => SplitPascal().Replace(value, " $1").Trim(),
    };

    [GeneratedRegex("(?<=[a-z0-9])([A-Z])")]
    private static partial Regex SplitPascal();
}
