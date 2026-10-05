//! Whole-session tests that span the scanner, the generator and the game.

use std::time::SystemTime;

use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::entities::Pos;
use crate::game::{Action, Game, Outcome};
use crate::testutil::{sample_project, snapshot};
use crate::{mapgen, scan};

/// Scan, generate, then play hundreds of turns of fighting and looting:
/// the directory must come out byte-for-byte, mtime-for-mtime unchanged.
#[test]
fn a_whole_session_never_writes_to_the_filesystem() {
    let tmp = sample_project();
    let before = snapshot(tmp.path());

    let root = tmp.path().canonicalize().expect("canonical");
    let tree = scan::scan(&root);
    let level = mapgen::build_level(&tree, mapgen::seed_for_path(&root));
    let mut game = Game::new(level, SystemTime::now());
    game.player.atk = 500; // kill everything we bump into: maximum "destruction"
    game.player.max_hp = 100_000;
    game.player.hp = 100_000;

    let mut rng = ChaCha8Rng::seed_from_u64(5);
    for _ in 0..400 {
        let action = match rng.random_range(0..10) {
            0 => Action::Search,
            1 => Action::Wait,
            2 if !game.player.inventory.is_empty() => Action::Use(0),
            _ => Action::Move(rng.random_range(-1..=1), rng.random_range(-1..=1)),
        };
        game.act(action);
        // Teleport next to a random monster now and then so fights really happen.
        if rng.random_bool(0.1) {
            if let Some(m) = game.monsters.first() {
                game.player.pos = Pos::new(m.pos.x + 1, m.pos.y);
            }
        }
    }
    assert!(game.summary().turns > 100, "a real session happened");
    assert_eq!(snapshot(tmp.path()), before);
}

/// Balance guard: with every file fresh (a just-cloned repo, the harshest
/// case) even a naive stair-diving bot must be able to win some runs, or
/// the game has become unwinnable.
#[test]
fn the_game_stays_winnable_when_every_file_is_fresh() {
    use crate::entities::ItemKind;
    use crate::testutil::TempDir;
    use std::collections::{HashMap, VecDeque};

    let tmp = TempDir::new();
    let exts = [
        "rs", "py", "js", "md", "json", "txt", "toml", "png", "zip", "lock", "sh", "",
    ];
    let mut n = 0u64;
    for (dir, count) in [
        ("", 22),
        ("src", 30),
        ("src/core", 18),
        ("tests", 20),
        ("docs", 14),
        ("scripts", 10),
        ("assets", 8),
    ] {
        for i in 0..count {
            n += 1;
            let ext = exts[(n as usize * 7 + i) % exts.len()];
            let name = if ext.is_empty() {
                format!("file{i}")
            } else {
                format!("file{i}.{ext}")
            };
            let rel = if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            };
            tmp.file(&rel, &vec![b'x'; 1 << ((n * 5 + i as u64 * 3) % 19)]);
        }
    }
    let root = tmp.path().canonicalize().expect("canonical");
    let tree = scan::scan(&root);

    // Next step of a shortest walk to `goal` (portals and walls respected).
    let step_towards = |g: &Game, goal: Pos| -> Option<Pos> {
        let start = g.player.pos;
        let mut prev: HashMap<Pos, Pos> = HashMap::from([(start, start)]);
        let mut queue = VecDeque::from([start]);
        while let Some(p) = queue.pop_front() {
            if p == goal {
                let mut cur = p;
                while prev[&cur] != start {
                    cur = prev[&cur];
                }
                return Some(cur);
            }
            for (dx, dy) in [
                (0, 1),
                (1, 0),
                (0, -1),
                (-1, 0),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let n = p.offset(dx, dy);
                if !prev.contains_key(&n)
                    && g.level.can_step(p, n, false)
                    && g.portal_at(n).is_none()
                {
                    prev.insert(n, p);
                    queue.push_back(n);
                }
            }
        }
        None
    };

    let (mut wins, mut deaths) = (0, 0);
    for seed in 0..100 {
        let mut g = Game::new(mapgen::build_level(&tree, seed), SystemTime::now());
        for _ in 0..3000 {
            if g.outcome != Outcome::Playing {
                break;
            }
            let me = g.player.pos;
            let gear = g
                .player
                .inventory
                .iter()
                .position(|i| matches!(i.kind, ItemKind::Weapon | ItemKind::Armor));
            let potion = g
                .player
                .inventory
                .iter()
                .position(|i| i.kind == ItemKind::Potion);
            let hurt = g.player.hp * 100 / g.player.max_hp < 45;
            // A neighbour we can actually hit (not around a solid corner).
            let target = g.monsters.iter().find(|m| {
                m.pos.dist(me) == 1
                    && (m.pos.x == me.x
                        || m.pos.y == me.y
                        || !(g.level.tile(Pos::new(me.x, m.pos.y)).opaque()
                            || g.level.tile(Pos::new(m.pos.x, me.y)).opaque()))
            });
            if let Some(i) = gear {
                g.act(Action::Use(i));
            } else if let (true, Some(i)) = (hurt, potion) {
                g.act(Action::Use(i));
            } else if let Some(m) = target {
                let (dx, dy) = (m.pos.x - me.x, m.pos.y - me.y);
                g.act(Action::Move(dx, dy));
            } else if let Some(next) = step_towards(&g, g.level.stairs) {
                g.act(Action::Move(next.x - me.x, next.y - me.y));
            } else {
                g.act(Action::Wait);
            }
        }
        match g.outcome {
            Outcome::Escaped => wins += 1,
            Outcome::Dead => deaths += 1,
            _ => {}
        }
    }
    println!("fresh-repo bot: {wins} wins, {deaths} deaths of 100");
    assert!(
        wins >= 2,
        "unwinnable: only {wins}/100 runs escaped ({deaths} deaths)"
    );
    assert!(deaths >= 5, "no danger at all: only {deaths}/100 runs died");
}
