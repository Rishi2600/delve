//! Entities: positions, monster classes and size tiers, items and loot,
//! and the player.

use std::time::{Duration, SystemTime};

use rand::{Rng, RngExt};

/// A tile coordinate on the level grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
}

impl Pos {
    pub const fn new(x: i32, y: i32) -> Pos {
        Pos { x, y }
    }

    pub const fn offset(self, dx: i32, dy: i32) -> Pos {
        Pos::new(self.x + dx, self.y + dy)
    }

    /// Chebyshev distance (diagonal steps cost 1, like movement does).
    pub fn dist(self, other: Pos) -> i32 {
        (self.x - other.x).abs().max((self.y - other.y).abs())
    }
}

// ---------------------------------------------------------------------------
// Monster classes (extension -> class)
// ---------------------------------------------------------------------------

/// How often a class acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    /// Two actions per turn.
    Fast,
    Normal,
    /// One action every other turn.
    Slow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// `.rs` — armored.
    Crab,
    /// `.py` — fast.
    Snake,
    /// `.js` / `.ts` — erratic.
    Goblin,
    /// `.md` / `.txt` — passes through walls.
    Ghost,
    /// `.json` / `.toml` / `.yaml` — slow and tanky.
    Golem,
    /// images — looks like an item until you are adjacent.
    Mimic,
    /// archives — drops lots of loot.
    Chest,
    /// everything else.
    Slime,
}

/// The extension -> class table. Extensions are matched case-insensitively.
const EXT_TABLE: &[(&[&str], Class)] = &[
    (&["rs"], Class::Crab),
    (&["py", "pyw"], Class::Snake),
    (&["js", "jsx", "mjs", "cjs", "ts", "tsx"], Class::Goblin),
    (&["md", "markdown", "txt", "rst"], Class::Ghost),
    (&["json", "toml", "yaml", "yml"], Class::Golem),
    (
        &[
            "png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico", "tif", "tiff",
        ],
        Class::Mimic,
    ),
    (
        &["zip", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar", "zst"],
        Class::Chest,
    ),
];

impl Class {
    /// Classify a file by its name. Dotfiles without an extension
    /// (`.env`, `.gitignore`) and extensionless files are slimes.
    pub fn for_file_name(file_name: &str) -> Class {
        let ext = match file_name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
            _ => return Class::Slime,
        };
        EXT_TABLE
            .iter()
            .find(|(exts, _)| exts.contains(&ext.as_str()))
            .map(|(_, class)| *class)
            .unwrap_or(Class::Slime)
    }

    pub fn glyph(self) -> char {
        match self {
            Class::Crab => 'C',
            Class::Snake => 'S',
            Class::Goblin => 'g',
            Class::Ghost => 'G',
            Class::Golem => 'H',
            Class::Mimic => 'M',
            Class::Chest => 'B',
            Class::Slime => 'j',
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Class::Crab => "crab",
            Class::Snake => "snake",
            Class::Goblin => "goblin",
            Class::Ghost => "ghost",
            Class::Golem => "golem",
            Class::Mimic => "mimic",
            Class::Chest => "chest-monster",
            Class::Slime => "slime",
        }
    }

    pub fn speed(self) -> Speed {
        match self {
            Class::Snake => Speed::Fast,
            Class::Golem => Speed::Slow,
            _ => Speed::Normal,
        }
    }

    /// Flat damage reduction (armor).
    pub fn armor(self) -> i32 {
        match self {
            Class::Crab => 2,
            Class::Golem => 1,
            _ => 0,
        }
    }

    pub fn passes_walls(self) -> bool {
        self == Class::Ghost
    }

    /// Moves in a random direction half of the time.
    pub fn erratic(self) -> bool {
        self == Class::Goblin
    }

    /// Never moves, only fights what is next to it.
    pub fn stationary(self) -> bool {
        matches!(self, Class::Chest | Class::Mimic)
    }

    /// Percentage multiplier on tier HP.
    fn hp_percent(self) -> i32 {
        match self {
            Class::Golem => 160,
            Class::Chest => 130,
            Class::Ghost => 70,
            Class::Mimic => 120,
            _ => 100,
        }
    }
}

// ---------------------------------------------------------------------------
// Size tiers (file size -> HP / attack)
// ---------------------------------------------------------------------------

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SizeTier {
    /// < 1 KiB
    Rat,
    /// < 16 KiB
    Small,
    /// < 256 KiB
    Medium,
    /// < 1 MiB
    Large,
    /// < 10 MiB
    Huge,
    /// >= 10 MiB
    Boss,
}

/// Log-scale tier table: file size -> tier.
pub fn size_tier(size: u64) -> SizeTier {
    match size {
        s if s < KIB => SizeTier::Rat,
        s if s < 16 * KIB => SizeTier::Small,
        s if s < 256 * KIB => SizeTier::Medium,
        s if s < MIB => SizeTier::Large,
        s if s < 10 * MIB => SizeTier::Huge,
        _ => SizeTier::Boss,
    }
}

impl SizeTier {
    pub fn index(self) -> i32 {
        self as i32
    }

    pub fn adjective(self) -> &'static str {
        match self {
            SizeTier::Rat => "tiny",
            SizeTier::Small => "small",
            SizeTier::Medium => "sturdy",
            SizeTier::Large => "large",
            SizeTier::Huge => "huge",
            SizeTier::Boss => "BOSS",
        }
    }

    /// Base (hp, attack, xp) for the tier.
    fn base(self) -> (i32, i32, u32) {
        match self {
            SizeTier::Rat => (4, 1, 3),
            SizeTier::Small => (8, 2, 6),
            SizeTier::Medium => (16, 3, 12),
            SizeTier::Large => (28, 5, 25),
            SizeTier::Huge => (48, 8, 55),
            SizeTier::Boss => (100, 13, 150),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub hp: i32,
    pub atk: i32,
    pub def: i32,
    pub xp: u32,
}

/// Combine tier and class (and the secret-room bonus) into final stats.
/// Monsters from secret rooms (`hidden`) are tougher: +50% HP, +2 attack,
/// double XP.
pub fn monster_stats(class: Class, tier: SizeTier, hidden: bool) -> Stats {
    let (hp, atk, xp) = tier.base();
    let mut hp = (hp * class.hp_percent() / 100).max(1);
    let mut atk = atk;
    let mut xp = xp;
    if hidden {
        hp = hp * 3 / 2;
        atk += 2;
        xp *= 2;
    }
    Stats {
        hp,
        atk,
        def: class.armor(),
        xp,
    }
}

// ---------------------------------------------------------------------------
// Awareness (modification time -> initial state)
// ---------------------------------------------------------------------------

const WEEK: Duration = Duration::from_secs(7 * 24 * 3600);
const YEAR: Duration = Duration::from_secs(365 * 24 * 3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonsterState {
    /// Untouched for a year or more: wakes when you come within 3 tiles.
    Asleep,
    /// Neither fresh nor ancient: wanders until you come close.
    Idle,
    /// Modified in the last 7 days: awake and hunting.
    Hunting,
}

/// Pick the starting state from a file's modification time.
pub fn initial_state(modified: Option<SystemTime>, now: SystemTime) -> MonsterState {
    let Some(modified) = modified else {
        return MonsterState::Idle;
    };
    // A modification time in the future counts as "just now".
    let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
    if age < WEEK {
        MonsterState::Hunting
    } else if age >= YEAR {
        MonsterState::Asleep
    } else {
        MonsterState::Idle
    }
}

// ---------------------------------------------------------------------------
// Spawns and monsters
// ---------------------------------------------------------------------------

/// What the map generator knows about a file.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// A monster placement decided by the map generator.
#[derive(Debug, Clone)]
pub struct Spawn {
    pub pos: Pos,
    pub file: FileInfo,
    /// Lives in a secret room (ignored entry): tougher, better loot.
    pub hidden: bool,
}

impl Spawn {
    pub fn class(&self) -> Class {
        Class::for_file_name(&self.file.name)
    }

    pub fn tier(&self) -> SizeTier {
        size_tier(self.file.size)
    }
}

#[derive(Debug, Clone)]
pub struct Monster {
    pub file: FileInfo,
    pub class: Class,
    pub tier: SizeTier,
    pub pos: Pos,
    pub hp: i32,
    pub max_hp: i32,
    pub atk: i32,
    pub def: i32,
    pub xp: u32,
    pub state: MonsterState,
    pub hidden: bool,
    /// A mimic that still looks like an item.
    pub disguised: bool,
    /// Turn counter used to pace slow monsters.
    pub ticks: u32,
}

impl Monster {
    pub fn from_spawn(spawn: &Spawn, now: SystemTime) -> Monster {
        let class = spawn.class();
        let tier = spawn.tier();
        let stats = monster_stats(class, tier, spawn.hidden);
        Monster {
            file: spawn.file.clone(),
            class,
            tier,
            pos: spawn.pos,
            hp: stats.hp,
            max_hp: stats.hp,
            atk: stats.atk,
            def: stats.def,
            xp: stats.xp,
            state: initial_state(spawn.file.modified, now),
            hidden: spawn.hidden,
            disguised: class == Class::Mimic,
            ticks: 0,
        }
    }

    /// e.g. `sturdy crab 'main.rs'`.
    pub fn label(&self) -> String {
        format!(
            "{} {} '{}'",
            self.tier.adjective(),
            self.class.name(),
            self.file.name
        )
    }
}

// ---------------------------------------------------------------------------
// Items and loot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// Heals `power` HP.
    Potion,
    /// Magic mapping: reveals the (non-secret) layout of the level.
    Scroll,
    /// Equippable, `+power` attack.
    Weapon,
    /// Equippable, `+power` defense.
    Armor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub kind: ItemKind,
    pub power: i32,
}

impl ItemKind {
    pub fn glyph(self) -> char {
        match self {
            ItemKind::Potion => '!',
            ItemKind::Scroll => '?',
            ItemKind::Weapon => '/',
            ItemKind::Armor => '[',
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            ItemKind::Potion => "Potion",
            ItemKind::Scroll => "Scroll",
            ItemKind::Weapon => "Blade",
            ItemKind::Armor => "Mail",
        }
    }
}

/// Roll one item named after `file_name`. `quality` grows with the foe's
/// tier (and secret rooms), improving the item's power.
pub fn roll_item<R: Rng + ?Sized>(file_name: &str, quality: i32, rng: &mut R) -> Item {
    let kind = match rng.random_range(0..100) {
        0..=39 => ItemKind::Potion,
        40..=59 => ItemKind::Scroll,
        60..=79 => ItemKind::Weapon,
        _ => ItemKind::Armor,
    };
    let power = match kind {
        ItemKind::Potion => 6 + 4 * quality,
        ItemKind::Scroll => 0,
        ItemKind::Weapon => 1 + quality / 2,
        ItemKind::Armor => 1 + quality / 3,
    };
    Item {
        name: format!("{} of {}", kind.prefix(), file_name),
        kind,
        power,
    }
}

/// Loot dropped by a defeated monster. Every item is named after the file.
/// Chest-monsters drop a lot; monsters from secret rooms always drop and
/// drop better items.
pub fn loot_for<R: Rng + ?Sized>(
    file_name: &str,
    class: Class,
    tier: SizeTier,
    hidden: bool,
    rng: &mut R,
) -> Vec<Item> {
    let mut count = if class == Class::Chest {
        3 + rng.random_range(0..=2)
    } else if rng.random_bool(0.5 + 0.1 * f64::from(tier.index())) {
        1
    } else {
        0
    };
    if hidden {
        count += 1;
    }
    if tier == SizeTier::Boss {
        count += 1;
    }
    let quality = tier.index() + if hidden { 2 } else { 0 };
    (0..count)
        .map(|_| roll_item(file_name, quality, rng))
        .collect()
}

/// Treasure lying around in a `.git` vault.
pub fn vault_loot<R: Rng + ?Sized>(rng: &mut R) -> Vec<Item> {
    (0..3).map(|_| roll_item(".git", 4, rng)).collect()
}

// ---------------------------------------------------------------------------
// The player
// ---------------------------------------------------------------------------

/// Inventory slots (one per letter a-z).
pub const MAX_INVENTORY: usize = 26;

#[derive(Debug, Clone)]
pub struct Player {
    pub pos: Pos,
    pub hp: i32,
    pub max_hp: i32,
    pub atk: i32,
    pub def: i32,
    pub level: u32,
    pub xp: u32,
    pub inventory: Vec<Item>,
    pub weapon: Option<Item>,
    pub armor: Option<Item>,
}

impl Player {
    pub fn new(pos: Pos) -> Player {
        Player {
            pos,
            hp: 40,
            max_hp: 40,
            atk: 4,
            def: 0,
            level: 1,
            xp: 0,
            inventory: Vec::new(),
            weapon: None,
            armor: None,
        }
    }

    pub fn total_atk(&self) -> i32 {
        self.atk + self.weapon.as_ref().map_or(0, |w| w.power)
    }

    pub fn total_def(&self) -> i32 {
        self.def + self.armor.as_ref().map_or(0, |a| a.power)
    }

    /// Cumulative XP needed to leave the current level.
    pub fn xp_for_next(&self) -> u32 {
        10 * self.level * (self.level + 1)
    }

    /// Add XP and level up as often as earned. Returns the levels gained.
    pub fn gain_xp(&mut self, amount: u32) -> u32 {
        self.xp += amount;
        let mut gained = 0;
        while self.xp >= self.xp_for_next() {
            self.level += 1;
            self.max_hp += 8;
            self.hp = (self.hp + 8).min(self.max_hp);
            self.atk += 1;
            if self.level.is_multiple_of(3) {
                self.def += 1;
            }
            gained += 1;
        }
        gained
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn extension_table_maps_to_the_specified_classes() {
        let cases = [
            ("main.rs", Class::Crab),
            ("app.py", Class::Snake),
            ("index.js", Class::Goblin),
            ("index.ts", Class::Goblin),
            ("README.md", Class::Ghost),
            ("notes.txt", Class::Ghost),
            ("package.json", Class::Golem),
            ("Cargo.toml", Class::Golem),
            ("ci.yaml", Class::Golem),
            ("ci.yml", Class::Golem),
            ("logo.png", Class::Mimic),
            ("photo.JPG", Class::Mimic),
            ("bundle.zip", Class::Chest),
            ("backup.tar.gz", Class::Chest),
            ("Makefile", Class::Slime),
            (".env", Class::Slime),
            (".gitignore", Class::Slime),
            ("weird.xyz", Class::Slime),
        ];
        for (name, class) in cases {
            assert_eq!(Class::for_file_name(name), class, "{name}");
        }
    }

    #[test]
    fn size_maps_to_log_scale_tiers() {
        let cases = [
            (0, SizeTier::Rat),
            (1023, SizeTier::Rat),
            (1024, SizeTier::Small),
            (16 * KIB - 1, SizeTier::Small),
            (16 * KIB, SizeTier::Medium),
            (256 * KIB - 1, SizeTier::Medium),
            (256 * KIB, SizeTier::Large),
            (MIB - 1, SizeTier::Large),
            (MIB, SizeTier::Huge),
            (10 * MIB - 1, SizeTier::Huge),
            (10 * MIB, SizeTier::Boss),
            (u64::MAX, SizeTier::Boss),
        ];
        for (size, tier) in cases {
            assert_eq!(size_tier(size), tier, "{size} bytes");
        }
    }

    #[test]
    fn bigger_tiers_are_strictly_tougher() {
        let tiers = [
            SizeTier::Rat,
            SizeTier::Small,
            SizeTier::Medium,
            SizeTier::Large,
            SizeTier::Huge,
            SizeTier::Boss,
        ];
        for pair in tiers.windows(2) {
            let a = monster_stats(Class::Slime, pair[0], false);
            let b = monster_stats(Class::Slime, pair[1], false);
            assert!(b.hp > a.hp && b.atk > a.atk && b.xp > a.xp);
        }
    }

    #[test]
    fn secret_room_monsters_are_tougher() {
        let normal = monster_stats(Class::Goblin, SizeTier::Medium, false);
        let hidden = monster_stats(Class::Goblin, SizeTier::Medium, true);
        assert!(hidden.hp > normal.hp && hidden.atk > normal.atk && hidden.xp > normal.xp);
    }

    #[test]
    fn class_modifiers_apply() {
        assert!(monster_stats(Class::Crab, SizeTier::Small, false).def > 0);
        assert!(
            monster_stats(Class::Golem, SizeTier::Small, false).hp
                > monster_stats(Class::Slime, SizeTier::Small, false).hp
        );
        assert_eq!(Class::Snake.speed(), Speed::Fast);
        assert_eq!(Class::Golem.speed(), Speed::Slow);
        assert!(Class::Ghost.passes_walls());
        assert!(Class::Goblin.erratic());
    }

    #[test]
    fn modification_time_sets_initial_state() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 365 * 24 * 3600);
        let ago = |d: Duration| Some(now - d);
        let day = Duration::from_secs(24 * 3600);
        assert_eq!(initial_state(ago(day), now), MonsterState::Hunting);
        assert_eq!(initial_state(ago(6 * day), now), MonsterState::Hunting);
        assert_eq!(initial_state(ago(8 * day), now), MonsterState::Idle);
        assert_eq!(initial_state(ago(364 * day), now), MonsterState::Idle);
        assert_eq!(initial_state(ago(365 * day), now), MonsterState::Asleep);
        assert_eq!(initial_state(ago(900 * day), now), MonsterState::Asleep);
        assert_eq!(initial_state(None, now), MonsterState::Idle);
        // Future mtimes (clock skew) are treated as fresh.
        assert_eq!(
            initial_state(Some(now + 5 * day), now),
            MonsterState::Hunting
        );
    }

    #[test]
    fn loot_is_named_after_the_file() {
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        for _ in 0..50 {
            let loot = loot_for("README.md", Class::Ghost, SizeTier::Large, true, &mut rng);
            assert!(!loot.is_empty(), "secret-room monsters always drop");
            for item in loot {
                assert!(item.name.ends_with(" of README.md"), "{}", item.name);
            }
        }
        let scroll = Item {
            name: format!("{} of {}", ItemKind::Scroll.prefix(), "README.md"),
            kind: ItemKind::Scroll,
            power: 0,
        };
        assert_eq!(scroll.name, "Scroll of README.md");
    }

    #[test]
    fn chest_monsters_drop_lots_and_hidden_loot_is_better() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        for _ in 0..20 {
            let loot = loot_for("a.zip", Class::Chest, SizeTier::Small, false, &mut rng);
            assert!(loot.len() >= 3);
        }
        let mut rng = ChaCha8Rng::seed_from_u64(2);
        let weak: i32 = (0..200).map(|_| roll_item("x", 0, &mut rng).power).sum();
        let strong: i32 = (0..200).map(|_| roll_item("x", 6, &mut rng).power).sum();
        assert!(strong > weak);
    }

    #[test]
    fn loot_is_deterministic_for_a_seed() {
        let roll = |seed| {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            loot_for("a.rs", Class::Crab, SizeTier::Huge, true, &mut rng)
        };
        assert_eq!(roll(9), roll(9));
    }

    #[test]
    fn player_levels_up_and_grows() {
        let mut p = Player::new(Pos::new(0, 0));
        assert_eq!(p.gain_xp(19), 0);
        assert_eq!(p.level, 1);
        assert_eq!(p.gain_xp(1), 1);
        assert_eq!(p.level, 2);
        assert!(p.max_hp > 40 && p.atk > 4);
        // A huge windfall can grant several levels at once.
        let before = p.level;
        assert!(p.gain_xp(1000) >= 2);
        assert!(p.level > before + 1);
    }

    #[test]
    fn equipment_feeds_totals() {
        let mut p = Player::new(Pos::new(0, 0));
        p.weapon = Some(Item {
            name: "Blade of a".into(),
            kind: ItemKind::Weapon,
            power: 3,
        });
        p.armor = Some(Item {
            name: "Mail of a".into(),
            kind: ItemKind::Armor,
            power: 2,
        });
        assert_eq!(p.total_atk(), 7);
        assert_eq!(p.total_def(), 2);
    }
}
