//! Game state and turn logic: movement, combat, XP, searching, monster AI,
//! fog of war and line of sight. Nothing here touches the filesystem —
//! killing and looting only ever change in-game state.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::entities::{
    loot_for, Class, Item, ItemKind, Monster, MonsterState, Player, Pos, Speed, MAX_INVENTORY,
};
use crate::mapgen::{Level, RoomKind, Tile};

/// How far you can see outside a lit room.
pub const VIEW_RADIUS: i32 = 9;
/// Hunting monsters track you along walkable paths up to this long.
const HUNT_RANGE: u16 = 30;
/// Monsters farther than this (in tiles) from you are frozen, to keep turns cheap.
const ACTIVE_RANGE: i32 = 40;
/// Sleeping monsters wake when you come this close.
const WAKE_RANGE: i32 = 3;
/// Idle monsters start hunting when you come this close.
const NOTICE_RANGE: i32 = 7;
const MAX_LOG: usize = 500;
const DIRS: [(i32, i32); 8] = [
    (0, -1),
    (1, 0),
    (0, 1),
    (-1, 0),
    (1, -1),
    (1, 1),
    (-1, 1),
    (-1, -1),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Playing,
    Dead,
    Escaped,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Step (or attack) in a direction.
    Move(i32, i32),
    Wait,
    /// Search the eight surrounding tiles for secret doors.
    Search,
    /// Use or equip the inventory item at this index.
    Use(usize),
    Quit,
}

/// What the end screen reports.
#[derive(Debug, Clone)]
pub struct Summary {
    pub rooms_explored: usize,
    pub rooms_total: usize,
    pub files_slain: u32,
    /// Name and size of the biggest foe defeated.
    pub biggest_foe: Option<(String, u64)>,
    /// The real path of the room you ended the run in.
    pub final_path: PathBuf,
    pub killer: Option<String>,
    pub secrets_found: u32,
    pub turns: u32,
    pub level: u32,
}

pub struct Game {
    pub level: Level,
    pub player: Player,
    pub monsters: Vec<Monster>,
    pub floor_items: Vec<(Pos, Item)>,
    /// Tiles ever seen (fog of war).
    pub seen: Vec<bool>,
    /// Tiles in view right now.
    pub visible: Vec<bool>,
    pub messages: Vec<String>,
    pub turn: u32,
    pub outcome: Outcome,

    rooms_visited: Vec<bool>,
    current_room: usize,
    files_slain: u32,
    biggest_foe: Option<(String, u64)>,
    secrets_found: u32,
    killer: Option<String>,
    final_path: PathBuf,

    visible_list: Vec<usize>,
    portal_set: HashSet<Pos>,
    dist: Vec<u16>,
    dist_stamp: Vec<u32>,
    epoch: u32,
    rng: ChaCha8Rng,
}

impl Game {
    pub fn new(level: Level, now: SystemTime) -> Game {
        let tiles = level.tiles.len();
        let monsters = level
            .spawns
            .iter()
            .map(|s| Monster::from_spawn(s, now))
            .collect();
        let mut game = Game {
            player: Player::new(level.start),
            monsters,
            floor_items: level.items.clone(),
            seen: vec![false; tiles],
            visible: vec![false; tiles],
            messages: Vec::new(),
            turn: 0,
            outcome: Outcome::Playing,
            rooms_visited: vec![false; level.rooms.len()],
            current_room: 0,
            files_slain: 0,
            biggest_foe: None,
            secrets_found: 0,
            killer: None,
            final_path: level.rooms[0].path.clone(),
            visible_list: Vec::new(),
            portal_set: level.portals.iter().map(|p| p.pos).collect(),
            dist: vec![0; tiles],
            dist_stamp: vec![0; tiles],
            epoch: 0,
            rng: ChaCha8Rng::seed_from_u64(level.seed ^ 0x9e37_79b9_7f4a_7c15),
            level,
        };
        game.enter_room_if_needed();
        game.update_view();
        game
    }

    // ----- queries ---------------------------------------------------------

    fn idx(&self, p: Pos) -> Option<usize> {
        self.level
            .in_bounds(p)
            .then(|| (p.y * self.level.width + p.x) as usize)
    }

    pub fn is_visible(&self, p: Pos) -> bool {
        self.idx(p).is_some_and(|i| self.visible[i])
    }

    pub fn is_seen(&self, p: Pos) -> bool {
        self.idx(p).is_some_and(|i| self.seen[i])
    }

    pub fn monster_at(&self, p: Pos) -> Option<usize> {
        self.monsters.iter().position(|m| m.pos == p)
    }

    pub fn item_at(&self, p: Pos) -> Option<&Item> {
        self.floor_items
            .iter()
            .find(|(q, _)| *q == p)
            .map(|(_, item)| item)
    }

    pub fn portal_at(&self, p: Pos) -> Option<&str> {
        self.level
            .portals
            .iter()
            .find(|portal| portal.pos == p)
            .map(|portal| portal.name.as_str())
    }

    /// The room you are in (or the last one you were in, when in a corridor).
    pub fn current_room_name(&self) -> &str {
        &self.level.rooms[self.current_room].name
    }

    /// The real filesystem path of the current room.
    pub fn current_path(&self) -> &Path {
        &self.level.rooms[self.current_room].path
    }

    pub fn current_room_kind(&self) -> RoomKind {
        self.level.rooms[self.current_room].kind
    }

    pub fn summary(&self) -> Summary {
        Summary {
            rooms_explored: self.rooms_visited.iter().filter(|&&v| v).count(),
            rooms_total: self.level.rooms.len(),
            files_slain: self.files_slain,
            biggest_foe: self.biggest_foe.clone(),
            final_path: self.final_path.clone(),
            killer: self.killer.clone(),
            secrets_found: self.secrets_found,
            turns: self.turn,
            level: self.player.level,
        }
    }

    fn log(&mut self, text: impl Into<String>) {
        self.messages.push(text.into());
        if self.messages.len() > MAX_LOG {
            self.messages.drain(..MAX_LOG / 2);
        }
    }

    // ----- actions ---------------------------------------------------------

    /// Perform one player action; time-consuming actions run the monsters' turn.
    pub fn act(&mut self, action: Action) {
        if self.outcome != Outcome::Playing {
            return;
        }
        let spent = match action {
            Action::Quit => {
                self.outcome = Outcome::Quit;
                self.final_path = self.current_path().to_path_buf();
                false
            }
            Action::Wait => true,
            Action::Search => {
                self.search();
                true
            }
            Action::Use(i) => self.use_item(i),
            Action::Move(dx, dy) => self.move_player(dx, dy),
        };
        if spent && self.outcome == Outcome::Playing {
            self.end_turn();
        }
    }

    fn move_player(&mut self, dx: i32, dy: i32) -> bool {
        let from = self.player.pos;
        let to = from.offset(dx, dy);

        if let Some(i) = self.monster_at(to) {
            if self.can_strike(from, to) {
                self.player_attack(i);
                return true;
            }
            return false;
        }
        if let Some(name) = self.portal_at(to) {
            let msg = format!(
                "A portal to '{name}' shimmers here. It leads outside the dungeon; you do not follow."
            );
            self.log(msg);
            return false;
        }
        if !self.level.can_step(from, to, false) {
            return false;
        }
        self.player.pos = to;
        self.pick_up();
        if self.level.tile(to) == Tile::Stairs {
            self.outcome = Outcome::Escaped;
            self.final_path = self.current_path().to_path_buf();
            self.log("You find the exit stairs and climb out of the directory. You escaped!");
            return false;
        }
        self.enter_room_if_needed();
        true
    }

    /// Can a creature at `from` strike a target at the adjacent `to`?
    /// (No attacking around solid corners.)
    fn can_strike(&self, from: Pos, to: Pos) -> bool {
        if from.dist(to) != 1 {
            return false;
        }
        if from.x != to.x && from.y != to.y {
            let solid = |p: Pos| self.level.tile(p).opaque();
            if solid(Pos::new(from.x, to.y)) || solid(Pos::new(to.x, from.y)) {
                return false;
            }
        }
        true
    }

    fn player_attack(&mut self, i: usize) {
        let bonus = self.rng.random_range(0..=2);
        let m = &mut self.monsters[i];
        if m.disguised {
            m.disguised = false;
            let label = m.label();
            self.log(format!("It was a mimic! You strike the {label}."));
        }
        let m = &mut self.monsters[i];
        m.state = MonsterState::Hunting;
        let dmg = (self.player.total_atk() + bonus - m.def).max(1);
        m.hp -= dmg;
        let label = m.label();
        if m.hp > 0 {
            self.log(format!("You hit the {label} for {dmg}."));
            return;
        }
        self.log(format!("You slay the {label}!"));
        let m = self.monsters.remove(i);
        self.files_slain += 1;
        let bigger = self
            .biggest_foe
            .as_ref()
            .is_none_or(|(_, size)| m.file.size > *size);
        if bigger {
            self.biggest_foe = Some((m.file.name.clone(), m.file.size));
        }
        if self.player.gain_xp(m.xp) > 0 {
            let level = self.player.level;
            self.log(format!("Welcome to level {level}! You feel stronger."));
        }
        let loot = loot_for(&m.file.name, m.class, m.tier, m.hidden, &mut self.rng);
        for item in loot {
            let spot = self.free_item_spot(m.pos);
            self.log(format!("The {} drops {}.", m.class.name(), item.name));
            self.floor_items.push((spot, item));
        }
    }

    /// The nearest walkable, empty tile to `near` for dropped loot.
    fn free_item_spot(&self, near: Pos) -> Pos {
        for radius in 0..=3i32 {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx.abs().max(dy.abs()) != radius {
                        continue;
                    }
                    let p = near.offset(dx, dy);
                    let taken = p == self.player.pos
                        || self.portal_set.contains(&p)
                        || self.floor_items.iter().any(|(q, _)| *q == p);
                    if self.level.tile(p).walkable() && !taken {
                        return p;
                    }
                }
            }
        }
        near
    }

    fn pick_up(&mut self) {
        let here = self.player.pos;
        let mut i = 0;
        while i < self.floor_items.len() {
            if self.floor_items[i].0 != here {
                i += 1;
                continue;
            }
            if self.player.inventory.len() >= MAX_INVENTORY {
                self.log("Your pack is full.");
                break;
            }
            let (_, item) = self.floor_items.remove(i);
            self.log(format!("You pick up {}.", item.name));
            self.player.inventory.push(item);
        }
    }

    fn search(&mut self) {
        let chance = (35 + 10 * self.player.level as i32).min(90);
        let mut found = 0;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let p = self.player.pos.offset(dx, dy);
                if self.level.tile(p) == Tile::SecretDoor && self.rng.random_range(0..100) < chance
                {
                    self.level.set_tile(p, Tile::Door);
                    found += 1;
                }
            }
        }
        if found > 0 {
            self.secrets_found += found;
            self.log("You find a secret door!");
        } else {
            self.log("You search, but find nothing.");
        }
    }

    /// Returns true if the action took a turn.
    fn use_item(&mut self, i: usize) -> bool {
        if i >= self.player.inventory.len() {
            return false;
        }
        let item = self.player.inventory.remove(i);
        match item.kind {
            ItemKind::Potion => {
                let before = self.player.hp;
                self.player.hp = (self.player.hp + item.power).min(self.player.max_hp);
                let healed = self.player.hp - before;
                self.log(format!("You drink {}: +{healed} HP.", item.name));
            }
            ItemKind::Scroll => {
                self.reveal_reachable();
                self.log(format!(
                    "You read {}: the dungeon's layout is burned into your mind.",
                    item.name
                ));
            }
            ItemKind::Weapon => {
                self.log(format!("You wield {} (+{} attack).", item.name, item.power));
                if let Some(old) = self.player.weapon.replace(item) {
                    self.player.inventory.push(old);
                }
            }
            ItemKind::Armor => {
                self.log(format!("You don {} (+{} defense).", item.name, item.power));
                if let Some(old) = self.player.armor.replace(item) {
                    self.player.inventory.push(old);
                }
            }
        }
        true
    }

    /// Magic mapping: reveal everything reachable without finding a secret
    /// door (secret rooms stay hidden).
    fn reveal_reachable(&mut self) {
        let mut reached: HashSet<Pos> = HashSet::new();
        let mut queue = VecDeque::from([self.player.pos]);
        reached.insert(self.player.pos);
        while let Some(p) = queue.pop_front() {
            for (dx, dy) in DIRS {
                let n = p.offset(dx, dy);
                if !reached.contains(&n) && self.level.can_step(p, n, false) {
                    reached.insert(n);
                    queue.push_back(n);
                }
            }
        }
        for p in reached {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if let Some(i) = self.idx(p.offset(dx, dy)) {
                        self.seen[i] = true;
                    }
                }
            }
        }
    }

    // ----- rooms -----------------------------------------------------------

    fn enter_room_if_needed(&mut self) {
        let p = self.player.pos;
        let Some(r) = self.level.room_at(p) else {
            return;
        };
        if !self.level.rooms[r].contains(p) {
            return;
        }
        self.current_room = r;
        if self.rooms_visited[r] {
            return;
        }
        self.rooms_visited[r] = true;
        let room = &self.level.rooms[r];
        let msg = match room.kind {
            RoomKind::Vault => format!("You step into the vault of {}: treasure glitters!", room.name),
            RoomKind::Sealed => format!(
                "The door of '{}' is sealed (permission denied). Nothing stirs within.",
                room.name
            ),
            RoomKind::Cache => "You squeeze into a hidden cache of ignored files.".to_string(),
            RoomKind::Normal if room.hidden => format!(
                "A hidden chamber: {}! Its denizens look tougher.",
                room.name
            ),
            RoomKind::Normal => format!("You enter {} ({}).", room.name, room.path.display()),
        };
        self.log(msg);
    }

    // ----- monster turn ----------------------------------------------------

    fn end_turn(&mut self) {
        self.turn += 1;
        self.monster_phase();
        if self.outcome == Outcome::Playing && self.turn % 8 == 0 && self.player.hp < self.player.max_hp
        {
            self.player.hp += 1;
        }
        self.update_view();
    }

    /// Breadth-first walking distance from the player, bounded by [`HUNT_RANGE`].
    fn compute_dist(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.dist_stamp.fill(0);
            self.epoch = 1;
        }
        let start = self.player.pos;
        let Some(si) = self.idx(start) else { return };
        self.dist_stamp[si] = self.epoch;
        self.dist[si] = 0;
        let mut queue = VecDeque::from([start]);
        while let Some(p) = queue.pop_front() {
            let d = self.dist[self.idx(p).unwrap_or(0)];
            if d >= HUNT_RANGE {
                continue;
            }
            for (dx, dy) in DIRS {
                let n = p.offset(dx, dy);
                let Some(ni) = self.idx(n) else { continue };
                if self.dist_stamp[ni] != self.epoch && self.level.can_step(p, n, false) {
                    self.dist_stamp[ni] = self.epoch;
                    self.dist[ni] = d + 1;
                    queue.push_back(n);
                }
            }
        }
    }

    fn dist_at(&self, p: Pos) -> Option<u16> {
        let i = self.idx(p)?;
        (self.dist_stamp[i] == self.epoch).then(|| self.dist[i])
    }

    fn monster_phase(&mut self) {
        self.compute_dist();
        let mut occupied: HashSet<Pos> = self.monsters.iter().map(|m| m.pos).collect();
        for i in 0..self.monsters.len() {
            if self.outcome != Outcome::Playing {
                break;
            }
            let m = &mut self.monsters[i];
            if m.pos.dist(self.player.pos) > ACTIVE_RANGE {
                continue;
            }
            let actions = match m.class.speed() {
                Speed::Fast => 2,
                Speed::Normal => 1,
                Speed::Slow => u32::from(m.ticks % 2 == 0),
            };
            m.ticks = m.ticks.wrapping_add(1);
            for _ in 0..actions {
                if self.outcome != Outcome::Playing {
                    break;
                }
                self.monster_act(i, &mut occupied);
            }
        }
    }

    fn monster_act(&mut self, i: usize, occupied: &mut HashSet<Pos>) {
        let player = self.player.pos;
        let (pos, class, state, disguised) = {
            let m = &self.monsters[i];
            (m.pos, m.class, m.state, m.disguised)
        };
        let dist = pos.dist(player);

        if disguised {
            if self.can_strike(pos, player) {
                self.monsters[i].disguised = false;
                self.monsters[i].state = MonsterState::Hunting;
                let label = self.monsters[i].label();
                self.log(format!("The item was a mimic! The {label} ambushes you."));
                self.monster_attack(i);
            }
            return;
        }

        match state {
            MonsterState::Asleep => {
                if dist <= WAKE_RANGE {
                    self.monsters[i].state = MonsterState::Hunting;
                    if self.is_visible(pos) {
                        let label = self.monsters[i].label();
                        self.log(format!("The {label} wakes up!"));
                    }
                }
                return;
            }
            MonsterState::Idle => {
                if dist <= NOTICE_RANGE && self.dist_at(pos).is_some() {
                    self.monsters[i].state = MonsterState::Hunting;
                } else {
                    if self.rng.random_bool(0.25) {
                        self.move_random(i, occupied);
                    }
                    return;
                }
            }
            MonsterState::Hunting => {}
        }

        if self.can_strike(pos, player) {
            self.monster_attack(i);
            return;
        }
        if class.stationary() {
            return;
        }

        let next = if class.erratic() && self.rng.random_bool(0.5) {
            self.random_step(class, pos, occupied)
        } else if class.passes_walls() {
            self.straight_step(class, pos, occupied)
        } else {
            self.gradient_step(class, pos, occupied)
        };
        if let Some(n) = next {
            Self::relocate(&mut self.monsters[i], n, occupied);
        } else if self.dist_at(pos).is_none() && self.rng.random_bool(0.3) {
            self.move_random(i, occupied);
        }
    }

    fn relocate(m: &mut Monster, to: Pos, occupied: &mut HashSet<Pos>) {
        occupied.remove(&m.pos);
        occupied.insert(to);
        m.pos = to;
    }

    fn move_random(&mut self, i: usize, occupied: &mut HashSet<Pos>) {
        let (class, pos) = (self.monsters[i].class, self.monsters[i].pos);
        if let Some(n) = self.random_step(class, pos, occupied) {
            Self::relocate(&mut self.monsters[i], n, occupied);
        }
    }

    fn can_move(&self, class: Class, from: Pos, to: Pos, occupied: &HashSet<Pos>) -> bool {
        if to == self.player.pos || occupied.contains(&to) || self.portal_set.contains(&to) {
            return false;
        }
        if class.passes_walls() {
            self.level.in_bounds(to)
        } else {
            self.level.can_step(from, to, false)
        }
    }

    fn random_step(&mut self, class: Class, pos: Pos, occupied: &HashSet<Pos>) -> Option<Pos> {
        let options: Vec<Pos> = DIRS
            .iter()
            .map(|&(dx, dy)| pos.offset(dx, dy))
            .filter(|&n| self.can_move(class, pos, n, occupied))
            .collect();
        if options.is_empty() {
            None
        } else {
            Some(options[self.rng.random_range(0..options.len())])
        }
    }

    /// Step along the walking-distance gradient towards the player.
    fn gradient_step(&self, class: Class, pos: Pos, occupied: &HashSet<Pos>) -> Option<Pos> {
        let here = self.dist_at(pos).unwrap_or(u16::MAX);
        DIRS.iter()
            .map(|&(dx, dy)| pos.offset(dx, dy))
            .filter(|&n| self.can_move(class, pos, n, occupied))
            .filter_map(|n| self.dist_at(n).map(|d| (d, n)))
            .filter(|&(d, _)| d < here)
            .min_by_key(|&(d, _)| d)
            .map(|(_, n)| n)
    }

    /// Ghosts drift straight at you, ignoring walls.
    fn straight_step(&self, class: Class, pos: Pos, occupied: &HashSet<Pos>) -> Option<Pos> {
        let sx = (self.player.pos.x - pos.x).signum();
        let sy = (self.player.pos.y - pos.y).signum();
        [(sx, sy), (sx, 0), (0, sy)]
            .into_iter()
            .filter(|&d| d != (0, 0))
            .map(|(dx, dy)| pos.offset(dx, dy))
            .find(|&n| self.can_move(class, pos, n, occupied))
    }

    fn monster_attack(&mut self, i: usize) {
        let bonus = self.rng.random_range(0..=1);
        let m = &self.monsters[i];
        let dmg = (m.atk + bonus - self.player.total_def()).max(1);
        let label = m.label();
        self.player.hp -= dmg;
        if self.player.hp > 0 {
            self.log(format!("The {label} hits you for {dmg}."));
            return;
        }
        self.player.hp = 0;
        self.killer = Some(label.clone());
        self.final_path = self.current_path().to_path_buf();
        self.outcome = Outcome::Dead;
        self.log(format!("The {label} hits you for {dmg}. You die..."));
    }

    // ----- fog of war / line of sight --------------------------------------

    /// Recompute what is visible: a lit room is seen whole when you stand in
    /// it, everything else by line of sight within [`VIEW_RADIUS`].
    pub fn update_view(&mut self) {
        for &i in &self.visible_list {
            self.visible[i] = false;
        }
        self.visible_list.clear();
        let p = self.player.pos;

        if let Some(r) = self.level.room_at(p) {
            let room = &self.level.rooms[r];
            if room.contains(p) {
                let (ox, oy, ow, oh) = room.outer();
                for y in oy..oy + oh {
                    for x in ox..ox + ow {
                        self.mark_visible(Pos::new(x, y));
                    }
                }
            }
        }
        let r2 = VIEW_RADIUS * VIEW_RADIUS + VIEW_RADIUS;
        for dy in -VIEW_RADIUS..=VIEW_RADIUS {
            for dx in -VIEW_RADIUS..=VIEW_RADIUS {
                if dx * dx + dy * dy <= r2 && self.has_line_of_sight(p, p.offset(dx, dy)) {
                    self.mark_visible(p.offset(dx, dy));
                }
            }
        }
    }

    fn mark_visible(&mut self, p: Pos) {
        if let Some(i) = self.idx(p) {
            if !self.visible[i] {
                self.visible[i] = true;
                self.visible_list.push(i);
            }
            self.seen[i] = true;
        }
    }

    /// Bresenham line; opaque tiles block, but the target itself may be opaque
    /// (that is how you see walls).
    fn has_line_of_sight(&self, from: Pos, to: Pos) -> bool {
        let (mut x, mut y) = (from.x, from.y);
        let dx = (to.x - x).abs();
        let dy = -(to.y - y).abs();
        let sx = if x < to.x { 1 } else { -1 };
        let sy = if y < to.y { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            if (x, y) == (to.x, to.y) {
                return true;
            }
            if (x, y) != (from.x, from.y) && self.level.tile(Pos::new(x, y)).opaque() {
                return false;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{FileInfo, Spawn};
    use crate::mapgen::Room;

    /// A walled, empty, rectangular room: interior (1,1)..(w-2,h-2).
    fn arena(w: i32, h: i32) -> Level {
        let mut tiles = vec![Tile::Floor; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                    tiles[(y * w + x) as usize] = Tile::Wall;
                }
            }
        }
        let stairs = Pos::new(w - 2, h - 2);
        tiles[(stairs.y * w + stairs.x) as usize] = Tile::Stairs;
        Level {
            width: w,
            height: h,
            tiles,
            room_map: vec![0; (w * h) as usize],
            rooms: vec![Room {
                id: 0,
                kind: RoomKind::Normal,
                name: "arena".into(),
                path: PathBuf::from("/arena"),
                x: 1,
                y: 1,
                w: w - 2,
                h: h - 2,
                depth: 0,
                hidden: false,
                secret_entry: false,
                parent: None,
                entries: 0,
            }],
            start: Pos::new(2, 2),
            stairs,
            spawns: vec![],
            items: vec![],
            portals: vec![],
            seed: 1,
        }
    }

    fn game(w: i32, h: i32) -> Game {
        Game::new(arena(w, h), SystemTime::now())
    }

    fn monster(name: &str, size: u64, pos: Pos, state: MonsterState) -> Monster {
        let spawn = Spawn {
            pos,
            file: FileInfo {
                name: name.into(),
                path: PathBuf::from("/arena").join(name),
                size,
                modified: None,
            },
            hidden: false,
            room: 0,
        };
        let mut m = Monster::from_spawn(&spawn, SystemTime::now());
        m.state = state;
        m
    }

    fn set_wall(g: &mut Game, p: Pos) {
        g.level.set_tile(p, Tile::Wall);
    }

    fn wait(g: &mut Game, turns: usize) {
        for _ in 0..turns {
            g.act(Action::Wait);
        }
    }

    #[test]
    fn new_game_starts_at_the_start_with_a_view() {
        let g = game(30, 12);
        assert_eq!(g.player.pos, g.level.start);
        assert_eq!(g.outcome, Outcome::Playing);
        assert!(g.is_visible(g.player.pos) && g.is_seen(g.player.pos));
        assert!(g.messages.iter().any(|m| m.contains("arena")));
    }

    #[test]
    fn walking_costs_a_turn_and_bumping_a_wall_does_not() {
        let mut g = game(30, 12);
        g.act(Action::Move(1, 0));
        assert_eq!((g.player.pos, g.turn), (Pos::new(3, 2), 1));
        g.player.pos = Pos::new(1, 1);
        let turn = g.turn;
        g.act(Action::Move(-1, 0));
        g.act(Action::Move(0, -1));
        assert_eq!((g.player.pos, g.turn), (Pos::new(1, 1), turn));
    }

    #[test]
    fn no_squeezing_through_diagonal_corners() {
        let mut g = game(30, 12);
        set_wall(&mut g, Pos::new(3, 2));
        set_wall(&mut g, Pos::new(2, 3));
        g.act(Action::Move(1, 1));
        assert_eq!(g.player.pos, Pos::new(2, 2));
    }

    #[test]
    fn combat_kills_awards_xp_drops_named_loot_and_tracks_stats() {
        let mut g = game(30, 12);
        g.monsters
            .push(monster("README.md", 900, Pos::new(3, 2), MonsterState::Asleep));
        g.player.atk = 100;
        g.act(Action::Move(1, 0));
        assert!(g.monsters.is_empty(), "one big hit slays a rat");
        assert!(g.player.xp > 0);
        let s = g.summary();
        assert_eq!(s.files_slain, 1);
        assert_eq!(s.biggest_foe, Some(("README.md".to_string(), 900)));
        assert!(g.messages.iter().any(|m| m.contains("You slay")));
        // Chests aside, loot (if any) is named after the file.
        assert!(g.floor_items.iter().all(|(_, i)| i.name.ends_with(" of README.md")));
    }

    #[test]
    fn biggest_foe_tracks_the_largest_kill() {
        let mut g = game(30, 12);
        g.player.atk = 1000;
        for (name, size, x) in [("a.txt", 50, 3), ("b.txt", 5_000_000, 4)] {
            g.monsters
                .push(monster(name, size, Pos::new(x, 2), MonsterState::Asleep));
        }
        g.act(Action::Move(1, 0));
        g.act(Action::Move(1, 0));
        g.act(Action::Move(1, 0));
        let foe = g.summary().biggest_foe;
        assert_eq!(foe, Some(("b.txt".to_string(), 5_000_000)));
    }

    #[test]
    fn chest_monsters_drop_a_lot() {
        let mut g = game(30, 12);
        g.monsters
            .push(monster("backup.tar.gz", 10, Pos::new(3, 2), MonsterState::Asleep));
        g.player.atk = 1000;
        g.act(Action::Move(1, 0));
        assert!(g.floor_items.len() >= 3);
    }

    #[test]
    fn walking_over_loot_picks_it_up() {
        let mut g = game(30, 12);
        let item = Item {
            name: "Potion of x".into(),
            kind: ItemKind::Potion,
            power: 10,
        };
        g.floor_items.push((Pos::new(3, 2), item.clone()));
        g.act(Action::Move(1, 0));
        assert!(g.floor_items.is_empty());
        assert_eq!(g.player.inventory, vec![item]);
    }

    #[test]
    fn search_reveals_adjacent_secret_doors_only() {
        let mut g = game(30, 12);
        let near = Pos::new(3, 3);
        let far = Pos::new(10, 5);
        g.level.set_tile(near, Tile::SecretDoor);
        g.level.set_tile(far, Tile::SecretDoor);
        assert!(!g.level.tile(near).walkable(), "secret doors act as walls");
        assert!(g.level.tile(near).opaque());
        for _ in 0..80 {
            g.act(Action::Search);
        }
        assert_eq!(g.level.tile(near), Tile::Door);
        assert_eq!(g.level.tile(far), Tile::SecretDoor);
        assert_eq!(g.summary().secrets_found, 1);
        assert!(g.level.tile(near).walkable());
    }

    #[test]
    fn search_chance_grows_with_level() {
        let attempts = |level: u32| {
            (0..400)
                .filter(|seed| {
                    let mut lvl = arena(30, 12);
                    lvl.seed = *seed;
                    let mut g = Game::new(lvl, SystemTime::now());
                    g.player.level = level;
                    g.level.set_tile(Pos::new(3, 3), Tile::SecretDoor);
                    g.act(Action::Search);
                    g.level.tile(Pos::new(3, 3)) == Tile::Door
                })
                .count()
        };
        assert!(attempts(8) > attempts(1));
    }

    #[test]
    fn sleepers_wake_within_three_tiles_only() {
        let mut g = game(40, 12);
        g.monsters
            .push(monster("old.rs", 10, Pos::new(9, 2), MonsterState::Asleep)); // 7 away
        g.monsters
            .push(monster("near.py", 10, Pos::new(5, 6), MonsterState::Asleep)); // 4 away
        wait(&mut g, 10);
        assert!(g.monsters.iter().all(|m| m.state == MonsterState::Asleep));
        g.monsters[1].pos = Pos::new(5, 4); // 3 away
        g.act(Action::Wait);
        assert_eq!(g.monsters[1].state, MonsterState::Hunting);
        assert_eq!(g.monsters[0].state, MonsterState::Asleep);
    }

    #[test]
    fn hunters_close_in_and_attack() {
        let mut g = game(40, 12);
        g.monsters
            .push(monster("fresh.txt", 10, Pos::new(12, 2), MonsterState::Hunting));
        let hp = g.player.hp;
        wait(&mut g, 14);
        assert!(g.player.hp < hp, "the hunter reached and hit us");
        assert!(g.monsters[0].pos.dist(g.player.pos) <= 1);
    }

    #[test]
    fn hunters_path_around_walls() {
        let mut g = game(40, 12);
        for y in 1..9 {
            set_wall(&mut g, Pos::new(6, y)); // wall with a gap at the bottom
        }
        g.monsters
            .push(monster("fresh.js", 5000, Pos::new(10, 2), MonsterState::Hunting));
        g.monsters[0].class = Class::Slime;
        let hp = g.player.hp;
        wait(&mut g, 40);
        assert!(g.player.hp < hp, "pathfinding got it round the wall");
    }

    #[test]
    fn ghosts_pass_through_walls() {
        let mut g = game(40, 12);
        for y in 1..11 {
            set_wall(&mut g, Pos::new(6, y)); // a solid wall, no gap
        }
        g.monsters
            .push(monster("ghost.md", 10, Pos::new(10, 2), MonsterState::Hunting));
        let hp = g.player.hp;
        wait(&mut g, 15);
        assert!(g.player.hp < hp, "the ghost drifted through the wall");

        let mut g = game(40, 12);
        for y in 1..11 {
            set_wall(&mut g, Pos::new(6, y));
        }
        g.monsters
            .push(monster("solid.rs", 10, Pos::new(10, 2), MonsterState::Hunting));
        wait(&mut g, 15);
        assert_eq!(g.player.hp, g.player.max_hp, "a crab cannot");
    }

    #[test]
    fn speed_classes_cover_different_ground() {
        let travelled = |name: &str| {
            let mut g = game(60, 6);
            g.player.pos = Pos::new(20, 2);
            g.monsters
                .push(monster(name, 10, Pos::new(50, 2), MonsterState::Hunting));
            wait(&mut g, 8);
            50 - g.monsters[0].pos.x
        };
        let (snake, slime, golem) = (travelled("a.py"), travelled("a.xyz"), travelled("a.json"));
        assert!(snake > slime && slime > golem, "{snake} {slime} {golem}");
        assert_eq!(slime, 8);
        assert_eq!(snake, 16);
        assert_eq!(golem, 4);
    }

    #[test]
    fn mimics_stay_disguised_until_you_are_adjacent() {
        let mut g = game(40, 12);
        g.monsters
            .push(monster("logo.png", 100, Pos::new(8, 2), MonsterState::Hunting));
        assert!(g.monsters[0].disguised);
        wait(&mut g, 5);
        assert!(g.monsters[0].disguised);
        assert_eq!(g.monsters[0].pos, Pos::new(8, 2), "mimics never move");
        for _ in 0..5 {
            g.act(Action::Move(1, 0));
        }
        assert!(!g.monsters.is_empty());
        assert!(!g.monsters[0].disguised, "revealed once we stood next to it");
    }

    #[test]
    fn death_ends_the_game_and_reports_the_real_path() {
        let mut g = game(30, 12);
        g.player.hp = 1;
        g.monsters
            .push(monster("boss.rs", 50_000_000, Pos::new(3, 2), MonsterState::Hunting));
        g.act(Action::Wait);
        assert_eq!(g.outcome, Outcome::Dead);
        let s = g.summary();
        assert_eq!(s.final_path, PathBuf::from("/arena"));
        assert!(s.killer.is_some_and(|k| k.contains("boss.rs")));
        let turn = g.turn;
        g.act(Action::Wait);
        assert_eq!(g.turn, turn, "no actions after death");
    }

    #[test]
    fn potions_heal_and_gear_equips() {
        let mut g = game(30, 12);
        g.player.hp = 5;
        g.player.inventory.push(Item {
            name: "Potion of a".into(),
            kind: ItemKind::Potion,
            power: 20,
        });
        g.player.inventory.push(Item {
            name: "Blade of b".into(),
            kind: ItemKind::Weapon,
            power: 3,
        });
        g.player.inventory.push(Item {
            name: "Mail of c".into(),
            kind: ItemKind::Armor,
            power: 2,
        });
        g.act(Action::Use(0));
        assert_eq!(g.player.hp, 25);
        g.act(Action::Use(0));
        g.act(Action::Use(0));
        assert_eq!((g.player.total_atk(), g.player.total_def()), (7, 2));
        assert!(g.player.inventory.is_empty());
        g.act(Action::Use(9));
    }

    /// A game whose "lit room" is tiny, so only line of sight applies at range.
    fn dim_game(w: i32, h: i32, edit: impl FnOnce(&mut Level)) -> Game {
        let mut lvl = arena(w, h);
        lvl.rooms[0].w = 4;
        edit(&mut lvl);
        Game::new(lvl, SystemTime::now())
    }

    #[test]
    fn fog_of_war_remembers_but_only_shows_the_present() {
        let mut g = dim_game(90, 12, |_| {});
        let far = Pos::new(60, 5);
        assert!(!g.is_seen(far), "unexplored at the start");
        g.player.pos = far;
        g.update_view();
        assert!(g.is_seen(far) && g.is_visible(far));
        g.player.pos = Pos::new(2, 2);
        g.update_view();
        assert!(g.is_seen(far), "remembered");
        assert!(!g.is_visible(far), "but no longer in view");
        assert!(g.is_visible(Pos::new(2, 2)));
    }

    #[test]
    fn walls_block_line_of_sight() {
        let g = dim_game(40, 12, |l| {
            for y in 1..11 {
                l.set_tile(Pos::new(6, y), Tile::Wall);
            }
        });
        assert!(g.is_visible(Pos::new(6, 2)), "the wall face is seen");
        assert!(!g.is_visible(Pos::new(8, 2)), "but not what is behind it");
    }

    #[test]
    fn scroll_maps_the_reachable_level_but_not_secret_rooms() {
        let mut g = dim_game(60, 12, |l| l.set_tile(Pos::new(30, 5), Tile::SecretDoor));
        assert!(!g.is_seen(Pos::new(55, 9)));
        g.player.inventory.push(Item {
            name: "Scroll of z".into(),
            kind: ItemKind::Scroll,
            power: 0,
        });
        g.act(Action::Use(0));
        assert!(g.is_seen(Pos::new(55, 9)), "far floor is mapped");
        assert!(g.is_seen(Pos::new(30, 5)), "the secret door looks like wall");
        assert_eq!(g.level.tile(Pos::new(30, 5)), Tile::SecretDoor);
    }

    #[test]
    fn stairs_escape_the_dungeon() {
        let mut g = game(10, 6);
        let s = g.level.stairs;
        g.player.pos = Pos::new(s.x - 1, s.y);
        g.act(Action::Move(1, 0));
        assert_eq!(g.outcome, Outcome::Escaped);
        assert_eq!(g.summary().final_path, PathBuf::from("/arena"));
    }

    #[test]
    fn portals_block_and_never_move_you() {
        let mut g = game(30, 12);
        g.level.portals.push(crate::mapgen::Portal {
            pos: Pos::new(3, 2),
            name: "link".into(),
            room: 0,
        });
        g.portal_set.insert(Pos::new(3, 2));
        let turn = g.turn;
        g.act(Action::Move(1, 0));
        assert_eq!((g.player.pos, g.turn), (Pos::new(2, 2), turn));
        assert!(g.messages.last().is_some_and(|m| m.contains("link")));
    }

    #[test]
    fn quit_ends_the_run() {
        let mut g = game(30, 12);
        g.act(Action::Quit);
        assert_eq!(g.outcome, Outcome::Quit);
    }

    #[test]
    fn simulation_is_deterministic() {
        let run = || {
            let mut lvl = arena(40, 12);
            lvl.seed = 77;
            let mut g = Game::new(lvl, SystemTime::now());
            g.monsters
                .push(monster("a.js", 5000, Pos::new(20, 5), MonsterState::Hunting));
            g.monsters
                .push(monster("b.xyz", 90, Pos::new(25, 8), MonsterState::Hunting));
            wait(&mut g, 30);
            (g.player.hp, g.monsters.iter().map(|m| m.pos).collect::<Vec<_>>())
        };
        assert_eq!(run(), run());
    }
}
