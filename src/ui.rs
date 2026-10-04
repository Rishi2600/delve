//! ratatui rendering: map, status bar, message log, inventory and the end screen.

use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::entities::{Class, Item, ItemKind, Monster, MonsterState, Pos};
use crate::game::{Action, Game, Outcome, Summary};
use crate::mapgen::Tile;

/// Smallest terminal the game will draw into.
pub const MIN_WIDTH: u16 = 40;
pub const MIN_HEIGHT: u16 = 14;

const LOG_HEIGHT: u16 = 6;
const STATUS_HEIGHT: u16 = 4;

/// Presentation state that is not part of the game itself.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub color: bool,
    pub inventory: bool,
}

pub const HELP: &str =
    " arrows/hjkl move · bump to attack · s search · i inventory · . wait · q quit ";

pub fn draw(frame: &mut Frame, game: &Game, view: &View) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let msg = format!(
            "Terminal too small: need {MIN_WIDTH}x{MIN_HEIGHT}, have {}x{}",
            area.width, area.height
        );
        frame.render_widget(Paragraph::new(msg).wrap(Wrap { trim: true }), area);
        return;
    }
    if game.outcome != Outcome::Playing {
        draw_end_screen(frame, game, view, area);
        return;
    }
    let [map_area, log_area, status_area] = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(LOG_HEIGHT),
        Constraint::Length(STATUS_HEIGHT),
    ])
    .areas(area);
    draw_map(frame, game, view, map_area);
    draw_log(frame, game, view, log_area);
    draw_status(frame, game, view, status_area);
    if view.inventory {
        draw_inventory(frame, game, view, area);
    }
}

fn paint(view: &View, fg: Color, extra: Modifier) -> Style {
    if view.color {
        Style::default().fg(fg).add_modifier(extra)
    } else {
        Style::default().add_modifier(extra)
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Translate one key press into game actions / view changes.
pub fn handle_key(game: &mut Game, view: &mut View, key: KeyEvent) {
    // Raw mode swallows SIGINT, so honour Ctrl-C ourselves.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        game.act(Action::Quit);
        return;
    }

    if view.inventory {
        match key.code {
            // Item letters win over the close keys, so 'i' and 'q' can name items.
            KeyCode::Char(c)
                if c.is_ascii_lowercase()
                    && usize::from(c as u8 - b'a') < game.player.inventory.len() =>
            {
                game.act(Action::Use(usize::from(c as u8 - b'a')));
                view.inventory = false;
            }
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i' | 'q' | ' ') => {
                view.inventory = false
            }
            _ => {}
        }
        return;
    }

    let (dx, dy) = match key.code {
        KeyCode::Left | KeyCode::Char('h') => (-1, 0),
        KeyCode::Right | KeyCode::Char('l') => (1, 0),
        KeyCode::Up | KeyCode::Char('k') => (0, -1),
        KeyCode::Down | KeyCode::Char('j') => (0, 1),
        KeyCode::Char('y') => (-1, -1),
        KeyCode::Char('u') => (1, -1),
        KeyCode::Char('b') => (-1, 1),
        KeyCode::Char('n') => (1, 1),
        KeyCode::Char('.') => return game.act(Action::Wait),
        KeyCode::Char('s') => return game.act(Action::Search),
        KeyCode::Char('i') => {
            view.inventory = true;
            return;
        }
        KeyCode::Char('q') => return game.act(Action::Quit),
        _ => return,
    };
    game.act(Action::Move(dx, dy));
}

// ---------------------------------------------------------------------------
// Map
// ---------------------------------------------------------------------------

fn class_color(class: Class) -> Color {
    match class {
        Class::Crab => Color::Red,
        Class::Snake => Color::Green,
        Class::Goblin => Color::Yellow,
        Class::Ghost => Color::Cyan,
        Class::Golem => Color::Magenta,
        Class::Mimic => Color::Magenta,
        Class::Chest => Color::LightYellow,
        Class::Slime => Color::LightGreen,
    }
}

fn item_color(kind: ItemKind) -> Color {
    match kind {
        ItemKind::Potion => Color::LightMagenta,
        ItemKind::Scroll => Color::LightCyan,
        ItemKind::Weapon => Color::LightBlue,
        ItemKind::Armor => Color::LightYellow,
    }
}

/// Glyph and style of what a monster looks like right now.
fn monster_look(view: &View, m: &Monster) -> (char, Style) {
    if m.disguised {
        // A mimic poses as loot until you are adjacent.
        return (
            '!',
            paint(view, item_color(ItemKind::Potion), Modifier::empty()),
        );
    }
    let mut extra = Modifier::empty();
    if m.hidden {
        extra |= Modifier::BOLD;
    }
    if m.state == MonsterState::Asleep {
        extra |= Modifier::DIM;
    }
    (m.class.glyph(), paint(view, class_color(m.class), extra))
}

fn tile_look(view: &View, tile: Tile, lit: bool) -> (char, Style) {
    let dim = if lit {
        Modifier::empty()
    } else {
        Modifier::DIM
    };
    match tile {
        Tile::Rock => (' ', Style::default()),
        // A secret door is indistinguishable from a wall.
        Tile::Wall | Tile::SecretDoor => ('#', paint(view, Color::Gray, dim)),
        Tile::Floor => ('.', paint(view, Color::Gray, dim)),
        Tile::SealedFloor => (':', paint(view, Color::Red, dim)),
        Tile::Corridor => ('·', paint(view, Color::DarkGray, dim)),
        Tile::Door => ('+', paint(view, Color::Yellow, dim)),
        Tile::Stairs => ('>', paint(view, Color::LightGreen, Modifier::BOLD | dim)),
    }
}

fn draw_map(frame: &mut Frame, game: &Game, view: &View, area: Rect) {
    let root = game.level.rooms[0].name.as_str();
    let block = Block::bordered().title(format!(" delve · {root} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let monsters: HashMap<Pos, &Monster> = game.monsters.iter().map(|m| (m.pos, m)).collect();
    let mut items: HashMap<Pos, &Item> = HashMap::new();
    for (pos, item) in &game.floor_items {
        items.entry(*pos).or_insert(item);
    }
    let portals: HashSet<Pos> = game.level.portals.iter().map(|p| p.pos).collect();

    let (iw, ih) = (i32::from(inner.width), i32::from(inner.height));
    let player = game.player.pos;
    let cam_x = (player.x - iw / 2).clamp(0, (game.level.width - iw).max(0));
    let cam_y = (player.y - ih / 2).clamp(0, (game.level.height - ih).max(0));

    let buf = frame.buffer_mut();
    for sy in 0..ih {
        for sx in 0..iw {
            let p = Pos::new(cam_x + sx, cam_y + sy);
            if !game.is_seen(p) {
                continue;
            }
            let lit = game.is_visible(p);
            let (ch, style) = if p == player {
                let style = if view.color {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
                };
                ('@', style)
            } else if let Some(m) = monsters.get(&p).filter(|_| lit) {
                monster_look(view, m)
            } else if portals.contains(&p) {
                (
                    'O',
                    paint(
                        view,
                        Color::LightBlue,
                        if lit { Modifier::BOLD } else { Modifier::DIM },
                    ),
                )
            } else if let Some(item) = items.get(&p) {
                (
                    item.kind.glyph(),
                    paint(
                        view,
                        item_color(item.kind),
                        if lit {
                            Modifier::empty()
                        } else {
                            Modifier::DIM
                        },
                    ),
                )
            } else {
                tile_look(view, game.level.tile(p), lit)
            };
            let (x, y) = (inner.x + sx as u16, inner.y + sy as u16);
            buf[(x, y)].set_char(ch).set_style(style);
        }
    }
}

// ---------------------------------------------------------------------------
// Log, status, inventory
// ---------------------------------------------------------------------------

fn draw_log(frame: &mut Frame, game: &Game, view: &View, area: Rect) {
    let block = Block::bordered().title(" log ");
    let rows = usize::from(block.inner(area).height);
    let start = game.messages.len().saturating_sub(rows);
    let last = game.messages.len().saturating_sub(1);
    let lines: Vec<Line> = game.messages[start..]
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let newest = start + i == last;
            let style = if newest {
                paint(view, Color::White, Modifier::BOLD)
            } else {
                paint(view, Color::Gray, Modifier::DIM)
            };
            Line::styled(text.as_str(), style)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// `[#####-----]` style bar.
pub fn bar(current: i32, max: i32, width: usize) -> String {
    let max = max.max(1);
    let filled = ((current.clamp(0, max) as usize) * width).div_ceil(max as usize);
    let filled = filled.min(width);
    format!("[{}{}]", "#".repeat(filled), "-".repeat(width - filled))
}

/// Keep the tail of a long path, which is the interesting end.
pub fn fit_path(path: &str, width: usize) -> String {
    let len = path.chars().count();
    if len <= width {
        return path.to_string();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let tail: String = path.chars().skip(len - (width - 1)).collect();
    format!("…{tail}")
}

fn draw_status(frame: &mut Frame, game: &Game, view: &View, area: Rect) {
    let p = &game.player;
    let ratio = p.hp * 100 / p.max_hp.max(1);
    let hp_color = if ratio > 50 {
        Color::Green
    } else if ratio > 25 {
        Color::Yellow
    } else {
        Color::Red
    };
    let line1 = Line::from(vec![
        Span::raw("HP "),
        Span::styled(
            format!("{} {}/{}", bar(p.hp, p.max_hp, 12), p.hp, p.max_hp),
            paint(view, hp_color, Modifier::BOLD),
        ),
        Span::raw(format!(
            "  Lv {}  XP {}/{}  ATK {}  DEF {}  Turn {}",
            p.level,
            p.xp,
            p.xp_for_next(),
            p.total_atk(),
            p.total_def(),
            game.turn
        )),
    ]);

    let inner_width = usize::from(area.width.saturating_sub(2));
    let prefix = format!("{}: ", game.current_room_name());
    let path = game.current_path().display().to_string();
    let room_line = Line::from(vec![
        Span::styled(prefix.clone(), paint(view, Color::Yellow, Modifier::BOLD)),
        Span::raw(fit_path(
            &path,
            inner_width.saturating_sub(prefix.chars().count()),
        )),
    ]);

    let block = Block::bordered().title_bottom(Line::from(HELP).alignment(Alignment::Left));
    frame.render_widget(Paragraph::new(vec![line1, room_line]).block(block), area);
}

fn item_blurb(item: &Item) -> String {
    match item.kind {
        ItemKind::Potion => format!("heals {}", item.power),
        ItemKind::Scroll => "reveals the map".to_string(),
        ItemKind::Weapon => format!("+{} attack", item.power),
        ItemKind::Armor => format!("+{} defense", item.power),
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn draw_inventory(frame: &mut Frame, game: &Game, view: &View, area: Rect) {
    let p = &game.player;
    let mut lines: Vec<Line> = Vec::new();
    let equipped = |label: &str, item: &Option<Item>| match item {
        Some(i) => format!("{label}: {} ({})", i.name, item_blurb(i)),
        None => format!("{label}: —"),
    };
    lines.push(Line::styled(
        equipped("Wielding", &p.weapon),
        paint(view, Color::LightBlue, Modifier::empty()),
    ));
    lines.push(Line::styled(
        equipped("Wearing ", &p.armor),
        paint(view, Color::LightYellow, Modifier::empty()),
    ));
    lines.push(Line::raw(""));
    if p.inventory.is_empty() {
        lines.push(Line::styled(
            "Your pack is empty.",
            paint(view, Color::Gray, Modifier::DIM),
        ));
    }
    for (i, item) in p.inventory.iter().enumerate() {
        let letter = char::from(b'a' + i as u8);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{letter}) "),
                paint(view, Color::Yellow, Modifier::BOLD),
            ),
            Span::styled(
                item.kind.glyph().to_string(),
                paint(view, item_color(item.kind), Modifier::empty()),
            ),
            Span::raw(format!(" {} ({})", item.name, item_blurb(item))),
        ]));
    }
    let height = lines.len() as u16 + 2;
    let rect = centered(area, 64, height.min(area.height));
    let block = Block::bordered()
        .title(" inventory ")
        .title_bottom(" letter: use / equip · esc: close ");
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

// ---------------------------------------------------------------------------
// End screen
// ---------------------------------------------------------------------------

/// `1.5 MiB`, `300 B`, ...
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn end_title(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Dead => "YOU DIED",
        Outcome::Escaped => "YOU ESCAPED",
        _ => "YOU WALKED AWAY",
    }
}

/// The end-of-run report (shown on the end screen and echoed after exit).
pub fn summary_lines(outcome: Outcome, s: &Summary) -> Vec<String> {
    let mut lines = vec![
        format!("Level reached     : {}", s.level),
        format!(
            "Rooms explored    : {} / {}",
            s.rooms_explored, s.rooms_total
        ),
        format!("Files slain       : {}", s.files_slain),
        format!(
            "Biggest foe       : {}",
            s.biggest_foe
                .as_ref()
                .map_or("none".to_string(), |(name, size)| format!(
                    "{name} ({})",
                    human_size(*size)
                ))
        ),
        format!("Secret doors found: {}", s.secrets_found),
        format!("Turns             : {}", s.turns),
    ];
    match outcome {
        Outcome::Dead => {
            lines.push(format!(
                "Slain by          : {}",
                s.killer.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!("Died at           : {}", s.final_path.display()));
        }
        Outcome::Escaped => lines.push(format!("Escaped through   : {}", s.final_path.display())),
        _ => lines.push(format!("Left at           : {}", s.final_path.display())),
    }
    lines
}

fn draw_end_screen(frame: &mut Frame, game: &Game, view: &View, area: Rect) {
    let outcome = game.outcome;
    let color = match outcome {
        Outcome::Dead => Color::Red,
        Outcome::Escaped => Color::LightGreen,
        _ => Color::Gray,
    };
    let mut lines = vec![
        Line::styled(end_title(outcome), paint(view, color, Modifier::BOLD))
            .alignment(Alignment::Center),
        Line::raw(""),
    ];
    lines.extend(
        summary_lines(outcome, &game.summary())
            .into_iter()
            .map(Line::raw),
    );
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "press any key to leave the dungeon",
        paint(view, Color::Gray, Modifier::DIM),
    ));
    let rect = centered(area, area.width.min(86), lines.len() as u16 + 2);
    let block = Block::bordered();
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{FileInfo, Spawn};
    use crate::game::Action;
    use crate::testutil::arena;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::time::SystemTime;

    fn render(game: &Game, view: &View, w: u16, h: u16) -> (Vec<String>, Terminal<TestBackend>) {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        term.draw(|f| draw(f, game, view)).expect("draw");
        let buf = term.backend().buffer().clone();
        let rows = (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        (rows, term)
    }

    fn text(rows: &[String]) -> String {
        rows.join("\n")
    }

    fn monster(name: &str, size: u64, pos: Pos, state: MonsterState) -> Monster {
        let spawn = Spawn {
            pos,
            file: FileInfo {
                name: name.into(),
                size,
                modified: None,
            },
            hidden: false,
        };
        let mut m = Monster::from_spawn(&spawn, SystemTime::now());
        m.state = state;
        m
    }

    const VIEW: View = View {
        color: true,
        inventory: false,
    };

    fn game() -> Game {
        Game::new(arena(40, 14), SystemTime::now())
    }

    #[test]
    fn main_screen_shows_map_log_and_status() {
        let g = game();
        let (rows, _) = render(&g, &VIEW, 80, 30);
        let t = text(&rows);
        assert!(t.contains('@'), "the player");
        assert!(t.contains("HP "), "status bar");
        assert!(t.contains("50/50"));
        assert!(t.contains("Lv 1"));
        assert!(t.contains("/arena"), "the real path of the room");
        assert!(t.contains("You enter arena"), "message log");
        assert!(t.contains("search"), "help line");
        assert!(t.contains('>'), "stairs are in view");
    }

    #[test]
    fn secret_doors_look_like_walls() {
        let mut g = game();
        g.level.set_tile(Pos::new(3, 3), Tile::SecretDoor);
        g.update_view();
        let (_, term) = render(&g, &VIEW, 80, 30);
        // The map starts at screen (1, 1) inside its border.
        let cell = &term.backend().buffer()[(4, 4)];
        assert_eq!(cell.symbol(), "#", "drawn as a plain wall");
        assert_eq!(
            tile_look(&VIEW, Tile::Wall, true),
            tile_look(&VIEW, Tile::SecretDoor, true)
        );
    }

    #[test]
    fn mimics_pose_as_items_until_revealed() {
        let mut g = game();
        g.monsters.push(monster(
            "logo.png",
            10,
            Pos::new(6, 2),
            MonsterState::Hunting,
        ));
        g.update_view();
        let (rows, _) = render(&g, &VIEW, 80, 30);
        assert!(!text(&rows).contains('M'));
        g.monsters[0].disguised = false;
        let (rows, _) = render(&g, &VIEW, 80, 30);
        assert!(text(&rows).contains('M'));
    }

    #[test]
    fn monsters_are_hidden_outside_view_and_sleepers_are_dimmed() {
        let mut g = Game::new(
            {
                let mut l = arena(90, 14);
                l.rooms[0].w = 4;
                l
            },
            SystemTime::now(),
        );
        g.monsters.push(monster(
            "far.rs",
            10,
            Pos::new(60, 5),
            MonsterState::Hunting,
        ));
        g.monsters
            .push(monster("near.rs", 10, Pos::new(5, 2), MonsterState::Asleep));
        g.update_view();
        let (rows, term) = render(&g, &VIEW, 100, 30);
        assert_eq!(
            text(&rows).matches('C').count(),
            1,
            "only the nearby crab is seen"
        );
        let buf = term.backend().buffer();
        let crab = (0..100u16)
            .flat_map(|x| (0..30u16).map(move |y| (x, y)))
            .find(|&(x, y)| buf[(x, y)].symbol() == "C")
            .expect("crab drawn");
        assert!(buf[crab].modifier.contains(Modifier::DIM), "asleep => dim");
    }

    #[test]
    fn no_color_mode_uses_no_colors() {
        let mut g = game();
        g.monsters
            .push(monster("a.rs", 10, Pos::new(5, 2), MonsterState::Hunting));
        g.update_view();
        let plain = View {
            color: false,
            inventory: false,
        };
        let (_, term) = render(&g, &plain, 80, 30);
        let buf = term.backend().buffer();
        assert!(buf
            .content()
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset));
        let (_, term) = render(&g, &VIEW, 80, 30);
        assert!(term
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|c| c.fg != Color::Reset));
    }

    #[test]
    fn inventory_overlay_lists_items_with_letters() {
        let mut g = game();
        for (name, kind, power) in [
            ("Potion of a.rs", ItemKind::Potion, 10),
            ("Scroll of README.md", ItemKind::Scroll, 0),
        ] {
            g.player.inventory.push(Item {
                name: name.into(),
                kind,
                power,
            });
        }
        let view = View {
            color: true,
            inventory: true,
        };
        let (rows, _) = render(&g, &view, 80, 30);
        let t = text(&rows);
        assert!(t.contains("a) ") && t.contains("Potion of a.rs") && t.contains("heals 10"));
        assert!(t.contains("b) ") && t.contains("Scroll of README.md"));
        assert!(t.contains("Wielding"));
        g.act(Action::Wait);
    }

    #[test]
    fn death_screen_reports_the_run() {
        let mut g = game();
        g.player.hp = 1;
        g.monsters.push(monster(
            "boss.rs",
            50_000_000,
            Pos::new(3, 2),
            MonsterState::Hunting,
        ));
        g.player.atk = 1000;
        g.monsters.push(monster(
            "victim.txt",
            2048,
            Pos::new(2, 3),
            MonsterState::Asleep,
        ));
        g.act(Action::Move(0, 1)); // slay the victim
        g.act(Action::Wait); // the boss finishes us
        assert_eq!(g.outcome, Outcome::Dead);
        let (rows, _) = render(&g, &VIEW, 100, 30);
        let t = text(&rows);
        assert!(t.contains("YOU DIED"));
        assert!(t.contains("Rooms explored    : 1 / 1"));
        assert!(t.contains("Files slain       : 1"));
        assert!(t.contains("victim.txt (2.0 KiB)"), "biggest foe defeated");
        assert!(t.contains("Died at           : /arena"), "the real path");
        assert!(t.contains("boss.rs"), "the killer");
    }

    #[test]
    fn escape_screen_has_its_own_title() {
        let mut g = game();
        g.outcome = Outcome::Escaped;
        let (rows, _) = render(&g, &VIEW, 100, 30);
        assert!(text(&rows).contains("YOU ESCAPED"));
    }

    #[test]
    fn tiny_terminals_get_a_message_instead_of_a_panic() {
        let g = game();
        let (rows, _) = render(&g, &VIEW, 20, 6);
        assert!(text(&rows).contains("Terminal too small"));
    }

    #[test]
    fn helpers_format_sizes_bars_and_paths() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(10 * 1024 * 1024), "10.0 MiB");
        assert_eq!(bar(5, 10, 10), "[#####-----]");
        assert_eq!(bar(0, 10, 4), "[----]");
        assert_eq!(bar(99, 10, 4), "[####]");
        assert_eq!(bar(1, 100, 10), "[#---------]", "any HP shows a sliver");
        assert_eq!(fit_path("/a/b", 10), "/a/b");
        assert_eq!(fit_path("/very/long/path/to/file", 10), "…h/to/file");
        assert_eq!(fit_path("/héllo/wörld/ünï", 8), "…rld/ünï");
        assert_eq!(fit_path("abc", 0), "");
    }

    #[test]
    fn camera_follows_the_player_on_big_maps() {
        let mut lvl = arena(200, 14);
        lvl.rooms[0].w = 198;
        let mut g = Game::new(lvl, SystemTime::now());
        g.player.pos = Pos::new(150, 6);
        g.update_view();
        let (rows, _) = render(&g, &VIEW, 60, 30);
        assert!(text(&rows).contains('@'), "the player stays on screen");
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn keys_map_to_moves_and_commands() {
        let mut g = game();
        let mut v = VIEW;
        for (code, expected) in [
            (KeyCode::Right, Pos::new(3, 2)),
            (KeyCode::Char('j'), Pos::new(3, 3)),
            (KeyCode::Left, Pos::new(2, 3)),
            (KeyCode::Char('k'), Pos::new(2, 2)),
            (KeyCode::Char('n'), Pos::new(3, 3)),
            (KeyCode::Char('y'), Pos::new(2, 2)),
            (KeyCode::Char('u'), Pos::new(3, 1)),
            (KeyCode::Char('b'), Pos::new(2, 2)),
            (KeyCode::Char('l'), Pos::new(3, 2)),
            (KeyCode::Char('h'), Pos::new(2, 2)),
        ] {
            handle_key(&mut g, &mut v, key(code));
            assert_eq!(g.player.pos, expected, "{code:?}");
        }
        let turn = g.turn;
        handle_key(&mut g, &mut v, key(KeyCode::Char('.')));
        assert_eq!(g.turn, turn + 1, "'.' waits");
        handle_key(&mut g, &mut v, key(KeyCode::Char('s')));
        assert_eq!(g.turn, turn + 2, "'s' searches");
        assert!(g.messages.last().is_some_and(|m| m.contains("search")));
        handle_key(&mut g, &mut v, key(KeyCode::Char('x')));
        assert_eq!(g.turn, turn + 2, "unknown keys do nothing");
        handle_key(&mut g, &mut v, key(KeyCode::Char('q')));
        assert_eq!(g.outcome, Outcome::Quit);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut g = game();
        let mut v = VIEW;
        handle_key(
            &mut g,
            &mut v,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert_eq!(g.outcome, Outcome::Quit);
    }

    #[test]
    fn inventory_keys_open_use_and_close() {
        let mut g = game();
        let mut v = VIEW;
        g.player.hp = 3;
        g.player.inventory.push(Item {
            name: "Potion of x".into(),
            kind: ItemKind::Potion,
            power: 10,
        });
        handle_key(&mut g, &mut v, key(KeyCode::Char('i')));
        assert!(v.inventory);
        // Movement keys are ignored while the pack is open.
        handle_key(&mut g, &mut v, key(KeyCode::Right));
        assert_eq!(g.player.pos, Pos::new(2, 2));
        handle_key(&mut g, &mut v, key(KeyCode::Char('z')));
        assert!(v.inventory, "an unknown letter keeps it open");
        handle_key(&mut g, &mut v, key(KeyCode::Char('a')));
        assert!(!v.inventory);
        assert_eq!(g.player.hp, 13);
        // 'q' closes the pack instead of quitting.
        handle_key(&mut g, &mut v, key(KeyCode::Char('i')));
        handle_key(&mut g, &mut v, key(KeyCode::Char('q')));
        assert!(!v.inventory);
        assert_eq!(g.outcome, Outcome::Playing);
        handle_key(&mut g, &mut v, key(KeyCode::Char('i')));
        handle_key(&mut g, &mut v, key(KeyCode::Esc));
        assert!(!v.inventory);
    }

    #[test]
    fn item_letters_may_collide_with_close_keys() {
        let mut g = game();
        let mut v = VIEW;
        for n in 0..9 {
            g.player.inventory.push(Item {
                name: format!("Scroll of {n}"),
                kind: ItemKind::Scroll,
                power: 0,
            });
        }
        handle_key(&mut g, &mut v, key(KeyCode::Char('i')));
        handle_key(&mut g, &mut v, key(KeyCode::Char('i'))); // the 9th item, not "close"
        assert_eq!(g.player.inventory.len(), 8);
    }
}
