//! One function per subcommand. Each prints its result and returns the exit code.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use amgi_core::search::Query;
use amgi_core::{BrowseCard, CardCollection, CardInfo, CardKind, Rating, StudyScope, format_interval};
use serde_json::json;

use crate::diff::{self, Source};
use crate::style::{self, blue, bold, dim, figure, green, red, yellow};
use crate::text::{self, bar, count, date_time, one_line, pad, sparkline, truncate, width};

type Result<T = ExitCode> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
pub struct Output {
    pub json: bool,
}

fn print_json(value: &impl serde::Serialize) -> Result {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(ExitCode::SUCCESS)
}

const DAY: i64 = 86_400;

/// `12 due · 5 new`, leaving out zeros.
fn due_and_new(due: u32, new: u32) -> String {
    [(due, "due", yellow as fn(String) -> String), (new, "new", blue)]
        .iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, label, paint)| paint(format!("{n} {label}")))
        .collect::<Vec<_>>()
        .join(&dim(" · "))
}

pub fn status(c: &CardCollection, out: Output, short: bool) -> Result {
    let today = c.today(StudyScope::All)?;
    // Top-level decks, as the menu bar lists them; their counts include subdecks.
    let decks: Vec<_> = c.decks()?.into_iter().filter(|d| !d.name.contains('/')).collect();
    let due: u32 = decks.iter().map(|d| d.due).sum();
    let new = today.new_available;
    if out.json {
        let decks: Vec<_> = decks.iter().filter(|d| d.due + d.new > 0).collect();
        return print_json(&json!({
            "due": due,
            "new": new,
            "reviewed": today.reviewed,
            "new_studied": today.new_studied,
            "decks": decks,
        }));
    }
    let left = due_and_new(due, new);
    if short {
        if !left.is_empty() {
            println!("{left}");
        }
        return Ok(ExitCode::SUCCESS);
    }
    let left = if left.is_empty() { green("All caught up") } else { bold(left) };
    println!("{left}{}", dim(format!(" · {} today", count(today.reviewed, "review", "reviews"))));
    let rows: Vec<_> = decks.iter().filter(|d| d.due + d.new > 0).collect();
    let name_width = rows.iter().map(|d| width(&d.title)).max().unwrap_or(0);
    for d in rows {
        println!("  {}  {}", pad(&d.title, name_width), due_and_new(d.due, d.new));
    }
    Ok(ExitCode::SUCCESS)
}

pub fn check(c: &CardCollection, out: Output) -> Result {
    let warnings = c.warnings();
    let code = if warnings.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    if out.json {
        print_json(&warnings)?;
        return Ok(code);
    }
    for w in &warnings {
        let place = match w.line {
            0 => w.path.clone(),
            line => format!("{}:{line}", w.path),
        };
        println!("{}{} {}", style::path(place), dim(":"), w.message);
    }
    if warnings.is_empty() {
        let cards = c.cards(StudyScope::All).len() as u32;
        println!("{}", green(format!("No problems in {}.", count(cards, "card", "cards"))));
    }
    Ok(code)
}

/// A schedule label drawn in its color: blue when new, yellow when due, red when
/// overdue, dim when it can wait. `padded` is the label padded to its column.
fn paint_label(label: &str, padded: String) -> String {
    match label {
        "new" => blue(padded),
        "due now" | "due today" => yellow(padded),
        l if l.starts_with("overdue") => red(padded),
        _ => dim(padded),
    }
}

/// `new`, `suspended`, `due now`, `due today`, `overdue 3d` or `in 2d`.
fn schedule_label(card: &BrowseCard, now: i64, tomorrow: i64) -> String {
    match card.due {
        _ if card.suspended => "suspended".into(),
        None => "new".into(),
        // Due for the whole study day it falls on, as `is:due` counts it.
        Some(due) if due > now && due < tomorrow => "due today".into(),
        Some(due) if due > now => format!("in {}", format_interval(due - now)),
        Some(due) if now - due < DAY => "due now".into(),
        Some(due) => format!("overdue {}", format_interval(now - due)),
    }
}

pub fn search(c: &CardCollection, out: Output, query: &str, view: Option<&str>, ids: bool) -> Result {
    let mut query = query.to_string();
    if let Some(name) = view {
        let views = c.saved_views()?;
        let Some(found) = views.iter().find(|v| v.name.eq_ignore_ascii_case(name.trim())) else {
            let names: Vec<_> = views.iter().map(|v| v.name.as_str()).collect();
            return Err(format!("no saved view named {name:?}; there are: {}", names.join(", ")).into());
        };
        query = format!("{} {query}", found.query);
    }
    let now = text::now();
    let matcher = Query::parse_with(&query, c.day_start(), &c.note_types());
    let cards: Vec<_> = c.cards(StudyScope::All).into_iter().filter(|card| matcher.matches(card)).collect();
    // Like grep: a search that finds nothing fails, for scripts.
    let code = if cards.is_empty() { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    if out.json {
        print_json(&cards)?;
        return Ok(code);
    }
    if ids {
        for card in &cards {
            println!("{}", card.id);
        }
        return Ok(code);
    }
    if cards.is_empty() {
        eprintln!("{}", dim("No cards match."));
        return Ok(code);
    }
    let tomorrow = c.day_start() + DAY;
    let rows: Vec<_> = cards
        .iter()
        .map(|card| (format!("{}:{}", card.path, card.line), schedule_label(card, now, tomorrow), one_line(&card.front)))
        .collect();
    let place_width = rows.iter().map(|r| width(&r.0)).max().unwrap_or(0);
    let label_width = rows.iter().map(|r| width(&r.1)).max().unwrap_or(0);
    let room = text::terminal_width().map(|w| w.saturating_sub(place_width + label_width + 4).max(20));
    for (place, label, front) in &rows {
        let front = room.map_or_else(|| front.clone(), |room| truncate(front, room));
        println!("{}  {}  {front}", style::path(pad(place, place_width)), paint_label(label, pad(label, label_width)));
    }
    Ok(code)
}

pub fn decks(c: &CardCollection, out: Output) -> Result {
    let decks = c.decks()?;
    if out.json {
        return print_json(&decks);
    }
    // Each deck indented under its parent, with its front-matter name if it has one.
    // Top-level decks in bold; a front-matter name dimmed after the file's.
    let names: Vec<(String, String)> = decks
        .iter()
        .map(|d| {
            let depth = d.name.matches('/').count();
            let last = d.name.rsplit('/').next().unwrap_or_default();
            let title = if d.title == last { String::new() } else { format!(" ({})", d.title) };
            let plain = format!("{}{last}{title}", "  ".repeat(depth));
            let name = if depth == 0 { bold(last) } else { last.to_string() };
            (plain, format!("{}{name}{}", "  ".repeat(depth), dim(title)))
        })
        .collect();
    let name_width = names.iter().map(|(plain, _)| width(plain)).max().unwrap_or(0).max(4);
    println!("{}", dim(format!("{}  {:>5}  {:>5}  {:>6}", pad("Deck", name_width), "Due", "New", "Total")));
    for (d, (plain, name)) in decks.iter().zip(&names) {
        let gap = " ".repeat(name_width - width(plain));
        println!("{name}{gap}  {}  {}  {}", figure(d.due, 5, yellow), figure(d.new, 5, blue), figure(d.total, 6, |s| s));
    }
    Ok(ExitCode::SUCCESS)
}

pub fn tags(c: &CardCollection, out: Output) -> Result {
    let icons = c.tag_icons()?;
    let rows: Vec<_> = c.tags().into_iter().map(|t| (icons.get(&t.name.to_lowercase()).cloned(), t)).collect();
    if out.json {
        let rows: Vec<_> =
            rows.iter().map(|(icon, t)| json!({ "tag": t.name, "cards": t.cards, "icon": icon })).collect();
        return print_json(&rows);
    }
    let tag_width = rows.iter().map(|(_, t)| width(&t.name) + 1).max().unwrap_or(0);
    for (icon, t) in &rows {
        let icon = icon.as_deref().map_or_else(|| "  ".to_string(), |i| pad(i, 2));
        let gap = " ".repeat(tag_width - width(&t.name) - 1);
        println!("{icon} {}{}{gap}  {}", dim("#"), t.name, dim(format!("{:>5}", t.cards)));
    }
    Ok(ExitCode::SUCCESS)
}

/// The cards `arg` names: an id, `path:line` on any line of a card, or a search. A cloze
/// makes several cards from one block, so a place or search can name all of them.
fn resolve<'a>(c: &CardCollection, cards: &'a [BrowseCard], arg: &str) -> Result<Vec<&'a BrowseCard>> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Err("name a card: its id, path:line, or a search".into());
    }
    if let Some(card) = cards.iter().find(|card| card.id == arg) {
        return Ok(vec![card]);
    }
    if let Some((path, line)) = arg.rsplit_once(':').and_then(|(p, l)| Some((p, l.parse::<u32>().ok()?))) {
        let path = collection_path(&c.root(), path);
        let start = cards.iter().filter(|card| card.path == path && card.line <= line).map(|card| card.line).max();
        let Some(start) = start else {
            return Err(format!("no card at {arg}").into());
        };
        return Ok(cards.iter().filter(|card| card.path == path && card.line == start).collect());
    }
    let matcher = Query::parse_with(arg, c.day_start(), &c.note_types());
    let found: Vec<_> = cards.iter().filter(|card| matcher.matches(card)).collect();
    match found.first() {
        None => Err(format!("no card matches {arg:?}").into()),
        Some(first) if found.iter().all(|card| card.path == first.path && card.line == first.line) => Ok(found),
        Some(_) => {
            let mut message = format!("{} cards match {arg:?}; pick one by id or path:line:", found.len());
            let places: Vec<_> = found.iter().take(10).map(|card| format!("{}:{}", card.path, card.line)).collect();
            let place_width = places.iter().map(|p| width(p)).max().unwrap_or(0);
            for (card, place) in found.iter().zip(&places) {
                let front = truncate(&one_line(&card.front), 60);
                message.push_str(&format!("\n  {}  {front}", style::path(pad(place, place_width))));
            }
            if found.len() > 10 {
                message.push_str(&dim(format!("\n  … and {} more", found.len() - 10)));
            }
            Err(message.into())
        }
    }
}

/// `path` relative to the collection root, whether given that way or relative to the current folder.
fn collection_path(root: &Path, path: &str) -> String {
    let given = Path::new(path);
    let absolute = std::env::current_dir().map(|cwd| cwd.join(given)).ok();
    let inside = absolute
        .and_then(|a| a.canonicalize().ok())
        .zip(root.canonicalize().ok())
        .and_then(|(a, root)| a.strip_prefix(&root).ok().map(Path::to_path_buf));
    match inside {
        Some(rel) => rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
        None => path.trim_start_matches("./").to_string(),
    }
}

pub fn info(c: &CardCollection, out: Output, arg: &str) -> Result {
    let cards = c.cards(StudyScope::All);
    let infos: Vec<CardInfo> = resolve(c, &cards, arg)?
        .into_iter()
        .filter_map(|card| c.card_info(card.id.clone()).transpose())
        .collect::<std::result::Result<_, _>>()?;
    if out.json {
        return print_json(&infos);
    }
    for (i, info) in infos.iter().enumerate() {
        if i > 0 {
            println!();
        }
        print_info(info);
    }
    Ok(ExitCode::SUCCESS)
}

/// The rating's name padded to `Again`'s width, in the color of its button in the app.
fn rating_name(r: Rating) -> String {
    match r {
        Rating::Again => red("Again"),
        Rating::Hard => yellow("Hard "),
        Rating::Good => green("Good "),
        Rating::Easy => blue("Easy "),
    }
}

/// Each line after the first indented to line up under the first.
fn indented(s: &str, indent: usize) -> String {
    text::plain(s).trim().lines().collect::<Vec<_>>().join(&format!("\n{}", " ".repeat(indent)))
}

fn print_info(info: &CardInfo) {
    let now = text::now();
    let kind = match info.kind {
        CardKind::Basic => "basic",
        CardKind::Cloze => "cloze",
    };
    let mut heading = vec![style::path(format!("{}:{}", info.path, info.line)), dim(kind)];
    if !info.tags.is_empty() {
        heading.push(dim(info.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")));
    }
    println!("{}", heading.join(&dim(" · ")));
    println!("{}", dim(format!("id {}", info.id)));
    println!();
    // A cloze's sides are its text with this card's gap hidden, then filled in.
    println!("{}  {}", dim("Q"), bold(indented(&info.front, 3)));
    println!("{}  {}", dim("A"), indented(&info.back, 3));
    println!();

    let field = |name: &str, value: String| println!("{}{value}", dim(pad(name, 13)));
    let schedule = match info.due {
        _ if info.suspended => dim("suspended"),
        None => blue("new, never studied"),
        Some(due) if due > now => format!("in {} {}", format_interval(due - now), dim(format!("({})", date_time(due)))),
        Some(due) => format!("{} {}", yellow("now"), dim(format!("since {}", date_time(due)))),
    };
    field("Due", schedule);
    if info.due.is_none() {
        return;
    }
    field("Reviews", format!("{}, {}", info.reps, count(info.lapses, "lapse", "lapses")));
    if let Some(s) = info.stability {
        field("Stability", format!("{} (until recall falls to 90%)", format_interval((s * DAY as f64) as i64)));
    }
    if let Some(d) = info.difficulty {
        field("Difficulty", format!("{d:.1} of 10"));
    }
    if let Some(r) = info.retrievability {
        field("Recall now", format!("{:.0}%", r * 100.0));
    }
    if let Some(t) = info.last_review {
        field("Last review", date_time(t));
    }
    if info.reviews.is_empty() {
        return;
    }
    println!();
    println!("{}", bold("History"));
    for r in &info.reviews {
        let gap = match r.elapsed_days {
            _ if r.was_new => "first".to_string(),
            d if d < 1.0 => "same day".to_string(),
            d => format!("after {d:.0}d"),
        };
        println!(
            "  {}  {}  {}  next in {}",
            dim(date_time(r.reviewed_at)),
            rating_name(r.rating),
            dim(format!("{gap:<11}")),
            format_interval(r.interval_secs)
        );
    }
}

pub fn stats(c: &CardCollection, out: Output, days: u32, deck: Option<&str>) -> Result {
    let days = days.max(1);
    let scope = match deck.map(|d| d.trim_matches('/')) {
        None => StudyScope::All,
        Some(name) => {
            // The deck or a folder of decks, as `deck:` in a search has them.
            let below = format!("{name}/");
            if !c.decks()?.iter().any(|d| d.name == name || d.name.starts_with(&below)) {
                return Err(format!("no deck named `{name}`; `amgi decks` lists them").into());
            }
            StudyScope::Deck { name: name.to_string() }
        }
    };
    let dashboard = c.dashboard(scope.clone(), days)?;
    let today = c.today(scope.clone())?;
    let deck = match &scope {
        StudyScope::All => None,
        StudyScope::Deck { name } => Some(name.as_str()),
    };
    if out.json {
        return print_json(&json!({ "deck": deck, "today": today, "dashboard": dashboard }));
    }
    let field = |name: &str, value: String| println!("{}{value}", dim(pad(name, 11)));
    if let Some(deck) = deck {
        field("Deck", bold(deck));
    }
    field("Today", format!("{}{}", bold(count(today.reviewed, "review", "reviews")), dim(format!(", {} new", today.new_studied))));
    field("Streak", bold(count(dashboard.streak, "day", "days")));
    field(
        "Retention",
        dashboard.retention.map_or(dim("not enough reviews yet"), |r| {
            format!("{}{}", bold(format!("{:.0}%", r * 100.0)), dim(" over the last 30 days"))
        }),
    );
    let cards = [
        blue(format!("{} new", dashboard.new_cards)),
        format!("{} young", dashboard.young_cards),
        green(format!("{} mature", dashboard.mature_cards)),
        dim(format!("{} suspended", dashboard.suspended_cards)),
    ];
    field("Cards", cards.join(&dim(" · ")));

    let total: u32 = dashboard.activity.iter().sum();
    let studied = dashboard.activity.iter().filter(|&&n| n > 0).count();
    println!();
    println!(
        "{}{}",
        bold(format!("Reviews in the last {}", count(days, "day", "days"))),
        dim(format!(": {total}, on {}", count(studied as u32, "day", "days")))
    );
    println!("  {}", blue(sparkline(&dashboard.activity)));
    // The first day under the line's start, and today under its end when there's room.
    let start = text::date(dashboard.activity_start);
    match (days as usize).checked_sub(start.len() + "today".len()) {
        Some(gap) => println!("  {}", dim(format!("{start}{}today", " ".repeat(gap)))),
        None => println!("  {}", dim(start)),
    }

    println!();
    println!("{}", bold(format!("Due in the next {} days", dashboard.forecast.len())));
    let max = dashboard.forecast.iter().copied().max().unwrap_or(0);
    let now = text::now();
    for (i, n) in dashboard.forecast.iter().enumerate() {
        let label = match i {
            0 => "Today".to_string(),
            1 => "Tomorrow".to_string(),
            i => format!("{} {}", text::weekday(now + i as i64 * DAY), &text::date(now + i as i64 * DAY)[5..]),
        };
        let label = if i == 0 { bold(pad(&label, 9)) } else { dim(pad(&label, 9)) };
        println!("  {label}  {}  {}", figure(*n, 4, |s| s), blue(bar(*n, max, 30)));
    }
    Ok(ExitCode::SUCCESS)
}

pub fn prune(c: &CardCollection, out: Output, trash: bool) -> Result {
    let root = c.root();
    let unused = amgi_core::media::unused(&root)?;
    if trash && !unused.is_empty() {
        let status = std::process::Command::new("/usr/bin/trash")
            .arg("-s")
            .args(unused.iter().map(|m| root.join(&m.path)))
            .status()?;
        if !status.success() {
            return Err("couldn't move every file to the Trash".into());
        }
    }
    if out.json {
        return print_json(&json!({ "files": unused, "trashed": trash }));
    }
    let bytes: u64 = unused.iter().map(|m| m.bytes).sum();
    for m in &unused {
        println!("{}", m.path);
    }
    let files = count(unused.len() as u32, "unused file", "unused files");
    match (unused.is_empty(), trash) {
        (true, _) => println!("{}", green("No unused media.")),
        (false, true) => println!("Moved {files} ({}) to the Trash.", text::size(bytes)),
        (false, false) => {
            let them = if unused.len() == 1 { "it" } else { "them" };
            println!(
                "{}{}",
                bold(format!("{files}, {}.", text::size(bytes))),
                dim(format!(" Run `amgi prune --trash` to move {them} to the Trash."))
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn diff(c: &CardCollection, out: Output, revs: &[String], staged: bool) -> Result {
    let head = || Source::Rev("HEAD".into());
    let current = if staged { Source::Staged } else { Source::Disk };
    let (from, to) = match revs {
        [] => (head(), current),
        [rev] => (Source::Rev(rev.clone()), current),
        [a, b] if !staged => (Source::Rev(a.clone()), Source::Rev(b.clone())),
        _ => return Err("--staged compares against one revision, not two".into()),
    };
    let root = c.root();
    let changes = diff::compare(diff::cards(&diff::files(&root, &from)?), diff::cards(&diff::files(&root, &to)?));
    if out.json {
        return print_json(&json!({
            "from": from.label(),
            "to": to.label(),
            "added": changes.added,
            "removed": changes.removed,
            "changed": changes.changed,
        }));
    }
    if changes.is_empty() {
        println!("{}", dim("No cards changed."));
        return Ok(ExitCode::SUCCESS);
    }
    // By file, then added, changed and removed, each in file order.
    let mut files: BTreeMap<&str, Vec<(char, &diff::Card, String)>> = BTreeMap::new();
    for card in &changes.added {
        files.entry(&card.path).or_default().push(('+', card, String::new()));
    }
    for changed in &changes.changed {
        let note = changed
            .changes
            .iter()
            .map(|&what| match what {
                "deck" => format!("moved from {}", changed.before.path),
                what => what.to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        files.entry(&changed.card.path).or_default().push(('~', &changed.card, note));
    }
    for card in &changes.removed {
        files.entry(&card.path).or_default().push(('-', card, String::new()));
    }
    let columns = text::terminal_width();
    for (i, (path, rows)) in files.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("{}", bold(style::path(path)));
        for (mark, card, note) in rows {
            let front = one_line(&card.front);
            let front = match columns {
                // Two spaces, the mark and a space before; two spaces before the note.
                Some(cols) => truncate(&front, cols.saturating_sub(4 + if note.is_empty() { 0 } else { width(note) + 2 }).max(10)),
                None => front,
            };
            let (mark, front) = match mark {
                '+' => (green('+'), front),
                '-' => (red('-'), dim(front)),
                _ => (yellow('~'), front),
            };
            match note.is_empty() {
                true => println!("  {mark} {front}"),
                false => println!("  {mark} {front}  {}", dim(format!("({note})"))),
            }
        }
    }
    let summary: Vec<_> = [
        (changes.added.len(), "added", green as fn(String) -> String),
        (changes.changed.len(), "changed", yellow),
        (changes.removed.len(), "removed", red),
    ]
    .iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, label, paint)| paint(format!("{n} {label}")))
    .collect();
    println!("\n{}", summary.join(&dim(" · ")));
    Ok(ExitCode::SUCCESS)
}
