//! One function per subcommand. Each prints its result and returns the exit code.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use amgi_core::lint;
use amgi_core::search::Query;
use amgi_core::{CardSummary, CardCollection, CardInfo, CardKind, Rating, Retention, StudyScope, format_interval};
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

/// `12 due · 2 learning · 5 new`, leaving out zeros.
fn left_today(due: u32, learning: u32, new: u32) -> String {
    [(due, "due", yellow as fn(String) -> String), (learning, "learning", red), (new, "new", blue)]
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
    // Not the decks' sum: they share the daily review limit.
    let due = today.due_available;
    let learning = today.learning;
    let new = today.new_available;
    if out.json {
        let decks: Vec<_> = decks.iter().filter(|d| d.due + d.learning + d.new > 0).collect();
        return print_json(&json!({
            "due": due,
            "learning": learning,
            "new": new,
            "reviewed": today.reviewed,
            "new_studied": today.new_studied,
            "study_secs": today.study_secs,
            "secs_left": today.secs_left,
            "decks": decks,
        }));
    }
    let left = left_today(due, learning, new);
    if short {
        if !left.is_empty() {
            println!("{left}");
        }
        return Ok(ExitCode::SUCCESS);
    }
    let about = match today.secs_left {
        Some(secs) if !left.is_empty() => dim(format!(" · about {}", text::study_time(secs.max(60)))),
        _ => String::new(),
    };
    let left = if left.is_empty() { green("All caught up") } else { bold(left) };
    let mut done = format!(" · {} today", count(today.reviewed, "review", "reviews"));
    if today.study_secs > 0 {
        done += &format!(", {}", text::study_time(today.study_secs));
    }
    println!("{left}{about}{}", dim(done));
    let rows: Vec<_> = decks.iter().filter(|d| d.due + d.learning + d.new > 0).collect();
    let name_width = rows.iter().map(|d| width(&d.title)).max().unwrap_or(0);
    for d in rows {
        println!("  {}  {}", pad(&d.title, name_width), left_today(d.due, d.learning, d.new));
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

pub fn lint(c: &CardCollection, out: Output, fix: bool) -> Result {
    let root = c.root();
    let config = lint::Config::load(&root)?;
    let problems = c.lint(&config, fix)?;
    let fixed = if fix { problems.iter().filter(|p| p.fixable).count() as u32 } else { 0 };
    let left: Vec<_> = problems.iter().filter(|p| !(fix && p.fixable)).collect();
    let code = if left.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    if out.json {
        print_json(&json!({ "problems": left, "fixed": fixed }))?;
        return Ok(code);
    }
    for p in &left {
        let place = match p.line {
            0 => p.path.clone(),
            line => format!("{}:{line}:{}", p.path, p.column),
        };
        println!("{}{} {} {}", style::path(place), dim(":"), p.message, dim(format!("({})", p.rule)));
    }
    if fixed > 0 {
        println!("{}", green(format!("Fixed {}.", count(fixed, "problem", "problems"))));
    }
    let fixable = left.iter().filter(|p| p.fixable).count() as u32;
    if left.is_empty() && fixed == 0 {
        println!("{}", green("No problems."));
    } else if fixable > 0 {
        let them = if fixable == left.len() as u32 { "them".to_string() } else { format!("{fixable} of them") };
        println!("{}{}", bold(count(left.len() as u32, "problem.", "problems.")), dim(format!(" Run `amgi lint --fix` to fix {them}.")));
    }
    Ok(code)
}

pub fn fmt(c: &CardCollection, out: Output, check: bool) -> Result {
    let formatted = c.format(!check)?;
    let left = if check { formatted.changed.len() } else { formatted.refused.len() };
    let code = if left == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    if out.json {
        print_json(&json!({ "changed": formatted.changed, "refused": formatted.refused, "written": !check }))?;
        return Ok(code);
    }
    for path in &formatted.changed {
        match formatted.refused.contains(path) {
            true => println!("{}{} {}", style::path(path), dim(":"), red("not formatted, because its cards would change")),
            false => println!("{}", style::path(path)),
        }
    }
    let files = (formatted.changed.len() - formatted.refused.len()) as u32;
    match (formatted.changed.is_empty(), check) {
        (true, _) => println!("{}", green("Every deck is formatted.")),
        (false, true) => println!(
            "{}{}",
            bold(count(formatted.changed.len() as u32, "file to format.", "files to format.")),
            dim(" Run `amgi fmt` to format them.")
        ),
        (false, false) if files > 0 => println!("{}", green(format!("Formatted {}.", count(files, "file", "files")))),
        (false, false) => {}
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
fn schedule_label(card: &CardSummary, now: i64, tomorrow: i64) -> String {
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

pub fn search(c: &CardCollection, out: Output, query: &str, ids: bool) -> Result {
    let now = text::now();
    let matcher = Query::parse_with(query, c.day_start(), &c.note_types());
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
    println!("{}", dim(format!("{}  {:>5}  {:>8}  {:>5}  {:>6}", pad("Deck", name_width), "Due", "Learning", "New", "Total")));
    for (d, (plain, name)) in decks.iter().zip(&names) {
        let gap = " ".repeat(name_width - width(plain));
        println!("{name}{gap}  {}  {}  {}  {}", figure(d.due, 5, yellow), figure(d.learning, 8, red), figure(d.new, 5, blue), figure(d.total, 6, |s| s));
    }
    Ok(ExitCode::SUCCESS)
}

pub fn tags(c: &CardCollection, out: Output) -> Result {
    let tags = c.tags();
    if out.json {
        let rows: Vec<_> = tags.iter().map(|t| json!({ "tag": t.name, "cards": t.cards })).collect();
        return print_json(&rows);
    }
    let tag_width = tags.iter().map(|t| width(&t.name) + 1).max().unwrap_or(0);
    for t in &tags {
        let gap = " ".repeat(tag_width - width(&t.name) - 1);
        println!("{}{}{gap}  {}", dim("#"), t.name, dim(format!("{:>5}", t.cards)));
    }
    Ok(ExitCode::SUCCESS)
}

/// The cards `arg` names: an id, `path:line` on any line of a card, or a search. A cloze
/// makes several cards from one block, so a place or search can name all of them.
fn resolve<'a>(c: &CardCollection, cards: &'a [CardSummary], arg: &str) -> Result<Vec<&'a CardSummary>> {
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

/// The deck or folder of decks named by `--deck`, as `deck:` in a search has them.
fn deck_scope(c: &CardCollection, deck: Option<&str>) -> Result<StudyScope> {
    Ok(match deck.map(|d| d.trim_matches('/')) {
        None => StudyScope::All,
        Some(name) => {
            let below = format!("{name}/");
            if !c.decks()?.iter().any(|d| d.name == name || d.name.starts_with(&below)) {
                return Err(format!("no deck named `{name}`; `amgi decks` lists them").into());
            }
            StudyScope::Deck { name: name.to_string() }
        }
    })
}

pub fn stats(c: &CardCollection, out: Output, days: u32, deck: Option<&str>, by_time: bool) -> Result {
    let days = days.max(1);
    let scope = deck_scope(c, deck)?;
    // At least the 30 days the daily average is taken over.
    let mut stats = c.stats(scope.clone(), days.max(30))?;
    let today = c.today(scope.clone())?;
    let recent = &stats.activity_secs[stats.activity_secs.len().saturating_sub(30)..];
    let studied: Vec<u32> = recent.iter().copied().filter(|&s| s > 0).collect();
    let daily_average = (!studied.is_empty()).then(|| studied.iter().sum::<u32>() / studied.len() as u32);
    let skip = stats.activity.len() - days as usize;
    stats.activity.drain(..skip);
    stats.activity_secs.drain(..skip);
    stats.activity_start += skip as i64 * DAY;
    let deck = match &scope {
        StudyScope::All => None,
        StudyScope::Deck { name } => Some(name.as_str()),
    };
    if out.json {
        return print_json(&json!({ "deck": deck, "today": today, "daily_average_secs": daily_average, "stats": stats }));
    }
    let field = |name: &str, value: String| println!("{}{value}", dim(pad(name, 11)));
    if let Some(deck) = deck {
        field("Deck", bold(deck));
    }
    let mut done = format!(", {} new", today.new_studied);
    if today.study_secs > 0 {
        done += &format!(", {}", text::study_time(today.study_secs));
    }
    field("Today", format!("{}{}", bold(count(today.reviewed, "review", "reviews")), dim(done)));
    field("Streak", bold(count(stats.streak, "day", "days")));
    // Only once some reviews were timed.
    let timed = stats.secs_per_review.is_some() || stats.activity_secs.iter().any(|&s| s > 0);
    if timed {
        let left = today.due_available + today.learning + today.new_available;
        if let Some(secs) = today.secs_left.filter(|_| left > 0) {
            field("Left", format!("about {}", bold(text::study_time(secs.max(60)))));
        }
        let pace = [
            daily_average.map(|s| format!("{} a day", text::study_time(s))),
            stats.secs_per_review.map(|s| format!("{} a review", text::review_time(s))),
        ];
        let pace: Vec<String> = pace.into_iter().flatten().collect();
        if !pace.is_empty() {
            field("Pace", format!("{}{}", pace.join(&dim(" · ")), dim(", over the last 30 days")));
        }
    }
    let cards = [
        blue(format!("{} new", stats.new_cards)),
        format!("{} young", stats.young_cards),
        green(format!("{} mature", stats.mature_cards)),
        dim(format!("{} suspended", stats.suspended_cards)),
    ];
    field("Cards", cards.join(&dim(" · ")));

    let (shaded, heading, total) = if by_time {
        let secs = &stats.activity_secs;
        (secs, "Time", text::study_time(secs.iter().sum()))
    } else {
        let reviews = &stats.activity;
        (reviews, "Reviews", reviews.iter().sum::<u32>().to_string())
    };
    let days_studied = shaded.iter().filter(|&&n| n > 0).count() as u32;
    println!();
    if days_studied == 0 {
        let none = if by_time { "No timed reviews" } else { "No reviews" };
        println!("{}", bold(format!("{none} in the last {}", count(days, "day", "days"))));
    } else {
        println!(
            "{}{}",
            bold(format!("{heading} in the last {}", count(days, "day", "days"))),
            dim(format!(": {total}, on {}", count(days_studied, "day", "days")))
        );
    }
    if days_studied > 0 && days <= SPARKLINE_DAYS && by_time {
        // A line per day, oldest first, with its time and a bar.
        let times: Vec<String> = shaded.iter().map(|&s| if s > 0 { text::study_time(s) } else { "–".into() }).collect();
        let time_width = times.iter().map(|t| width(t)).max().unwrap_or(0);
        let max = shaded.iter().copied().max().unwrap_or(0);
        let last = shaded.len() - 1;
        for (i, (&secs, time)) in shaded.iter().zip(&times).enumerate() {
            let day = stats.activity_start + i as i64 * DAY;
            let label = match last - i {
                0 => bold(pad("Today", 9)),
                1 => dim(pad("Yesterday", 9)),
                _ => dim(pad(&format!("{} {}", text::weekday(day), &text::date(day)[5..]), 9)),
            };
            let time = format!("{}{time}", " ".repeat(time_width - width(time)));
            let time = if secs > 0 { time } else { dim(time) };
            let bar = if secs > 0 { format!("  {}", blue(bar(secs, max, 30))) } else { String::new() };
            println!("  {label}  {time}{bar}");
        }
    } else if days_studied > 0 && days <= SPARKLINE_DAYS {
        println!("  {}", blue(sparkline(shaded)));
        // The first day under the line's start, and today under its end when there's room.
        let start = text::date(stats.activity_start);
        match (days as usize).checked_sub(start.len() + "today".len()) {
            Some(gap) => println!("  {}", dim(format!("{start}{}today", " ".repeat(gap)))),
            None => println!("  {}", dim(start)),
        }
    } else if days_studied > 0 {
        for line in calendar(shaded, stats.activity_start) {
            println!("  {line}");
        }
    }

    println!();
    println!("{}", bold("Retention"));
    println!("  {}", dim(format!("{:<10}  {:>6}  {:>6}  {:>6}  {:>7}", "", "Young", "Mature", "Total", "Reviews")));
    let r = &stats.retention;
    for (label, p) in [("Today", r.today), ("Yesterday", r.yesterday), ("Last week", r.week), ("Last month", r.month), ("Last year", r.year)] {
        let cell = |passed, of| match percent(passed, of) {
            Some(p) => format!("{p:>6}"),
            None => dim(format!("{:>6}", "–")),
        };
        println!(
            "  {}  {}  {}  {}  {}",
            dim(pad(label, 10)),
            cell(p.young_passed, p.young_passed + p.young_failed),
            cell(p.mature_passed, p.mature_passed + p.mature_failed),
            cell(p.young_passed + p.mature_passed, total_reviews(&p)),
            figure(total_reviews(&p), 7, |s| s)
        );
    }

    println!();
    println!("{}", bold(format!("Due in the next {} days", stats.forecast.len())));
    let max = stats.forecast.iter().copied().max().unwrap_or(0);
    let now = text::now();
    for (i, n) in stats.forecast.iter().enumerate() {
        let label = match i {
            0 => "Today".to_string(),
            1 => "Tomorrow".to_string(),
            i => format!("{} {}", text::weekday(now + i as i64 * DAY), &text::date(now + i as i64 * DAY)[5..]),
        };
        let label = if i == 0 { bold(pad(&label, 9)) } else { dim(pad(&label, 9)) };
        // Lined up after the longest bar.
        let bar = bar(*n, max, 30);
        let line = match stats.secs_per_due_card {
            Some(secs) if *n > 0 => {
                let time = text::study_time((secs * f64::from(*n)).round() as u32);
                format!("{}  {}", blue(pad(&bar, 30)), dim(format!("about {time}")))
            }
            _ => blue(bar),
        };
        println!("  {label}  {}  {line}", figure(*n, 4, |s| s));
    }
    Ok(ExitCode::SUCCESS)
}

/// Days `amgi forecast` shows one by one; longer stretches show a line per week.
const FORECAST_DAILY_DAYS: u32 = 31;

pub fn forecast(c: &CardCollection, out: Output, new_cards: u32, days: u32, deck: Option<&str>) -> Result {
    let days = days.max(1);
    let scope = deck_scope(c, deck)?;
    let w = c.workload(scope.clone(), new_cards, days)?;
    if out.json {
        let deck = match &scope {
            StudyScope::All => None,
            StudyScope::Deck { name } => Some(name.as_str()),
        };
        return print_json(&json!({ "deck": deck, "workload": w }));
    }

    // Cards to study each day: those studied before, and what the new ones add, them
    // included today. Seconds where reviews were timed.
    let added: Vec<f64> = w.added_reviews.iter().enumerate().map(|(i, n)| n + if i == 0 { f64::from(new_cards) } else { 0.0 }).collect();
    let secs = |i: usize| -> Option<f64> {
        let new = if i == 0 { f64::from(new_cards) * w.secs_per_new_card? } else { 0.0 };
        Some((w.reviews[i] + w.added_reviews[i]) * w.secs_per_due_card? + new)
    };
    let now = text::now();
    let day_label = |i: usize| match i {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        i => format!("{} {}", text::weekday(now + i as i64 * DAY), &text::date(now + i as i64 * DAY)[5..]),
    };
    // A row per day, or per week from today over longer stretches.
    let span = if days <= FORECAST_DAILY_DAYS { 1 } else { 7 };
    let rows: Vec<(String, f64, f64, Option<f64>)> = (0..days as usize)
        .step_by(span)
        .map(|start| {
            let range = start..(start + span).min(days as usize);
            let label = match span {
                1 => day_label(start),
                _ if start == 0 => "This week".to_string(),
                _ => format!("From {}", &text::date(now + start as i64 * DAY)[5..]),
            };
            let sum = |v: &[f64]| v[range.clone()].iter().sum::<f64>();
            let time = range.clone().map(secs).sum::<Option<f64>>();
            (label, sum(&w.reviews), sum(&added), time)
        })
        .collect();

    let mut heading = format!("Reviews in the next {}", count(days, "day", "days"));
    if new_cards > 0 {
        heading += &format!(", studying {} today", count(new_cards, "new card", "new cards"));
    }
    println!("{}", bold(heading));
    let max = rows.iter().map(|r| (r.1 + r.2).round() as u32).max().unwrap_or(0);
    for (i, (label, before, added, time)) in rows.iter().enumerate() {
        let total = (before + added).round() as u32;
        let added = added.round() as u32;
        let label = if i == 0 { bold(pad(label, 9)) } else { dim(pad(label, 9)) };
        let plus = if new_cards == 0 {
            String::new()
        } else if added == 0 {
            dim(format!("{:>6}", ""))
        } else {
            blue(format!("{:>6}", format!("+{added}")))
        };
        // Cards studied before, then the new ones' share, padded to line up the times.
        let len = bar(total, max, 30).chars().count();
        let new_len = bar(added, max, 30).chars().count().min(len);
        let bar = format!("{}{}{}", yellow("█".repeat(len - new_len)), blue("▒".repeat(new_len)), " ".repeat(30 - len));
        let time = match time {
            Some(t) if total > 0 => dim(format!("  about {}", text::study_time(t.round() as u32))),
            _ => String::new(),
        };
        println!("  {label}  {}{plus}  {bar}{time}", figure(total, 5, |s| s));
    }

    let before: f64 = w.reviews.iter().sum();
    let reviews: f64 = w.added_reviews.iter().sum();
    let total_time = |v: &[f64], new: u32| -> Option<f64> {
        let new = if new > 0 { f64::from(new) * w.secs_per_new_card? } else { 0.0 };
        Some(v.iter().sum::<f64>() * w.secs_per_due_card? + new)
    };
    let about = |secs: Option<f64>| secs.map(|s| format!(", about {}", text::study_time(s.round() as u32))).unwrap_or_default();
    println!();
    println!(
        "  {}{}",
        count(before.round() as u32, "review", "reviews"),
        dim(format!(" of cards studied before{}", about(total_time(&w.reviews, 0))))
    );
    if new_cards > 0 {
        println!(
            "  {}{}",
            blue(format!("+{}", count(reviews.round() as u32, "review", "reviews"))),
            dim(format!(
                " of the {} after today{}",
                count(new_cards, "new card", "new cards"),
                about(total_time(&w.added_reviews, new_cards))
            ))
        );
    }
    println!("  {}", dim("Simulated from each card's memory, without the daily review limit."));
    Ok(ExitCode::SUCCESS)
}

/// Longer stretches of days show as a calendar rather than one line.
const SPARKLINE_DAYS: u32 = 60;

/// Days as a grid, a column per week and a row per weekday from Monday, like the
/// contribution graph on GitHub, with month names above.
fn calendar(values: &[u32], start: i64) -> Vec<String> {
    const SHADES: [&str; 5] = ["·", "░", "▒", "▓", "█"];
    let levels = text::levels(values);
    let first = text::weekday_index(start);
    let weeks = (first + values.len()).div_ceil(7);
    let mut months = String::new();
    let mut last_month = "";
    for week in 0..weeks {
        // The week's first day in range.
        let day = (week * 7).saturating_sub(first);
        let month = text::month(start + day as i64 * DAY);
        if month != last_month && width(&months) <= week {
            months = pad(&months, week) + month;
        }
        last_month = month;
    }
    let mut lines = vec![format!("    {}", dim(months))];
    for row in 0..7 {
        let name = ["Mon", "", "Wed", "", "Fri", "", ""][row];
        let mut line = format!("{} ", dim(pad(name, 3)));
        for week in 0..weeks {
            let cell = (week * 7 + row).checked_sub(first).and_then(|day| levels.get(day));
            line += &match cell {
                None => " ".to_string(),
                Some(0) => dim(SHADES[0]),
                Some(&level) => blue(SHADES[level]),
            };
        }
        lines.push(line.trim_end().to_string());
    }
    lines
}

fn total_reviews(r: &Retention) -> u32 {
    r.young_passed + r.young_failed + r.mature_passed + r.mature_failed
}

/// `passed` of `of` as a percentage to one decimal; `None` without reviews.
fn percent(passed: u32, of: u32) -> Option<String> {
    (of > 0).then(|| format!("{:.1}%", f64::from(passed) * 100.0 / f64::from(of)))
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

pub fn diff(c: &CardCollection, out: Output, revs: &[String], staged: bool, check: bool, only_reviewed: bool) -> Result {
    let head = || Source::Rev("HEAD".into());
    let current = if staged { Source::Staged } else { Source::Disk };
    let (from, to) = match revs {
        [] => (head(), current),
        [rev] => (Source::Rev(rev.clone()), current),
        [a, b] if !staged => (Source::Rev(a.clone()), Source::Rev(b.clone())),
        _ => return Err("--staged compares against one revision, not two".into()),
    };
    let root = c.root();
    let mut changes = diff::compare(diff::cards(&diff::files(&root, &from)?), diff::cards(&diff::files(&root, &to)?));
    // History under a removed card's id, or its id from before media links were resolved.
    let ids: Vec<String> = changes.removed.iter().flat_map(|card| [Some(card.id.clone()), card.old_id.clone()]).flatten().collect();
    let counts = c.review_counts(&ids)?;
    let reviews = |card: &diff::Card| {
        [Some(&card.id), card.old_id.as_ref()].into_iter().flatten().filter_map(|id| counts.get(id)).sum::<u32>()
    };
    if only_reviewed {
        changes.added.clear();
        changes.changed.clear();
        changes.removed.retain(|card| reviews(card) > 0);
    }
    let reviewed = changes.removed.iter().filter(|card| reviews(card) > 0).count() as u32;
    let code = if check && reviewed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS };
    if out.json {
        let removed = changes
            .removed
            .iter()
            .map(|card| {
                let mut value = serde_json::to_value(card)?;
                value["reviews"] = json!(reviews(card));
                Ok(value)
            })
            .collect::<std::result::Result<Vec<_>, serde_json::Error>>()?;
        print_json(&json!({
            "from": from.label(),
            "to": to.label(),
            "added": changes.added,
            "removed": removed,
            "changed": changes.changed,
        }))?;
        return Ok(code);
    }
    if changes.is_empty() {
        let none = if only_reviewed { "No removed cards have review history." } else { "No cards changed." };
        println!("{}", dim(none));
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
        let note = match reviews(card) {
            0 => String::new(),
            n => count(n, "review", "reviews"),
        };
        files.entry(&card.path).or_default().push(('-', card, note));
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
    if !only_reviewed {
        println!("\n{}", summary.join(&dim(" · ")));
    } else {
        println!();
    }
    if reviewed > 0 {
        let cards = count(reviewed, "removed card has", "removed cards have");
        println!("{}", red(format!("{cards} review history, which no card will use.")));
    }
    Ok(code)
}
