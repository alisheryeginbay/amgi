//! An [AnkiConnect](https://git.sr.ht/~foosoft/anki-connect)-compatible API, so tools
//! made for Anki, such as Yomitan, can add cards to the collection and look them up.
//!
//! Anki's ideas map onto amgi's like this:
//! - A deck `Languages::Spanish` is the file `cards/Languages/Spanish.md`.
//! - A model is a note type in `note-types`, or `Basic` (Front, Back) for `Q:`/`A:`
//!   cards and `Cloze` (Text, Back Extra) for `C:` cards.
//! - A note is the block a card comes from, and its id the first of its cards' ids.
//!   Ids are amgi's card ids cut to 53 bits, so they survive as JavaScript numbers,
//!   and like amgi's they change when a card's front does.
//! - Fields arrive as HTML and are written as Markdown, as when importing from Anki.
//!
//! Answers follow the request's `version`: 5 and later wrap results as
//! `{"result": …, "error": null}`, earlier versions send results bare. Errors are
//! always `{"result": null, "error": "…"}`.

mod query;
pub mod server;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Map, Value, json};

use crate::anki::text::{self, Media};
use crate::parser::NoteTypeInfo;
use crate::search::fold;
use crate::{BrowseCard, CARDS_DIR, CardCollection, CardKind, MEDIA_DIR, NewCard, NewNote, StudyScope};

/// The AnkiConnect API version answered, the one current clients expect.
pub const VERSION: u64 = 6;
/// The port AnkiConnect listens on, which clients try by default.
pub const DEFAULT_PORT: u16 = 8765;

/// Told about what the API did, so the app can show it.
#[uniffi::export(with_foreign)]
pub trait AnkiConnectDelegate: Send + Sync {
    /// Cards were added or changed.
    fn cards_changed(&self);
    /// A client asked to see a note: `path` is its deck file, relative to the
    /// collection root.
    fn show(&self, path: String);
}

/// Answer one AnkiConnect request, given as JSON.
pub fn handle(collection: &CardCollection, request: &str, delegate: &dyn AnkiConnectDelegate) -> String {
    let mut api = Api { collection, delegate, changed: false };
    let reply = match serde_json::from_str::<Value>(request) {
        Ok(request) => api.request(&request),
        Err(e) => reply(VERSION, Err(format!("invalid request: {e}"))),
    };
    if api.changed {
        delegate.cards_changed();
    }
    reply.to_string()
}

fn reply(version: u64, result: Result<Value, String>) -> Value {
    match result {
        Ok(value) if version <= 4 => value,
        Ok(value) => json!({ "result": value, "error": null }),
        Err(error) => json!({ "result": null, "error": error }),
    }
}

/// The actions answered, for `apiReflect`.
const ACTIONS: &[&str] = &[
    "addNote",
    "addNotes",
    "apiReflect",
    "areSuspended",
    "canAddNotes",
    "canAddNotesWithErrorDetail",
    "cardsInfo",
    "deckNames",
    "deckNamesAndIds",
    "findCards",
    "findNotes",
    "getMediaDirPath",
    "getTags",
    "guiBrowse",
    "guiEditNote",
    "modelFieldNames",
    "modelNames",
    "modelNamesAndIds",
    "multi",
    "notesInfo",
    "requestPermission",
    "retrieveMediaFile",
    "storeMediaFile",
    "suspend",
    "sync",
    "unsuspend",
    "version",
];

struct Api<'a> {
    collection: &'a CardCollection,
    delegate: &'a dyn AnkiConnectDelegate,
    /// Whether cards were added or changed, to tell the delegate once at the end.
    changed: bool,
}

/// A card with what Anki knows it by.
struct Entry<'a> {
    card: &'a BrowseCard,
    note_id: i64,
    card_id: i64,
    /// Which of its note's cards it is, from 0.
    ord: u32,
    model: String,
    /// With `::` between a deck and the decks inside it, as Anki writes it.
    deck: String,
    /// Names and Markdown values.
    fields: Vec<(String, String)>,
    /// Each field as folded plain text, for searching.
    plain: Vec<String>,
}

/// The cards and note types as of one request.
struct Snapshot {
    cards: Vec<BrowseCard>,
    types: Vec<NoteTypeInfo>,
    day_start: i64,
}

impl Snapshot {
    fn entries(&self) -> Vec<Entry<'_>> {
        // Each note's id and how many of its cards came before.
        let mut notes: HashMap<(&str, u32), (i64, u32)> = HashMap::new();
        self.cards
            .iter()
            .map(|card| {
                let note = notes.entry((&card.path, card.line)).or_insert((number(&card.id), 0));
                let (note_id, ord) = *note;
                note.1 += 1;
                let (model, fields): (String, Vec<(String, String)>) = match &card.note_type {
                    Some(name) => {
                        let names = self.types.iter().find(|t| &t.name == name).map(|t| t.fields.clone());
                        (name.clone(), names.unwrap_or_default().into_iter().zip(card.fields.iter().cloned()).collect())
                    }
                    None => {
                        let names = match card.kind {
                            CardKind::Basic => BASIC,
                            CardKind::Cloze => CLOZE,
                        };
                        let fields = [card.front.clone(), card.back.clone()];
                        (names.0.into(), names.1.iter().map(|n| n.to_string()).zip(fields).collect())
                    }
                };
                Entry {
                    card,
                    note_id,
                    card_id: number(&card.id),
                    ord,
                    model,
                    deck: card.deck.replace('/', "::"),
                    plain: fields.iter().map(|(_, value)| plain(value)).collect(),
                    fields,
                }
            })
            .collect()
    }

    /// What a model is in amgi, and its fields.
    fn model(&self, name: &str) -> Result<(Model, Vec<String>), String> {
        if let Some(t) = self.types.iter().find(|t| t.name == name) {
            return Ok((Model::NoteType, t.fields.clone()));
        }
        let fields = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
        match name {
            n if n == BASIC.0 => Ok((Model::Basic, fields(BASIC.1))),
            n if n == CLOZE.0 => Ok((Model::Cloze, fields(CLOZE.1))),
            _ => Err(format!("model was not found: {name}")),
        }
    }

    fn model_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.types.iter().map(|t| t.name.clone()).collect();
        for builtin in [BASIC.0, CLOZE.0] {
            if !names.iter().any(|n| n == builtin) {
                names.push(builtin.into());
            }
        }
        names
    }
}

/// The models standing for `Q:`/`A:` and `C:` cards, with their fields.
const BASIC: (&str, &[&str]) = ("Basic", &["Front", "Back"]);
const CLOZE: (&str, &[&str]) = ("Cloze", &["Text", "Back Extra"]);

#[derive(Clone, Copy, PartialEq)]
enum Model {
    NoteType,
    Basic,
    Cloze,
}

/// A card or note id as a number: its top 53 bits, which a JavaScript number holds.
fn number(id: &str) -> i64 {
    (u64::from_str_radix(id, 16).unwrap_or(0) >> 11) as i64
}

/// Markdown as folded plain text, with escapes taken out.
fn plain(markdown: &str) -> String {
    let mut out = String::new();
    let mut chars = markdown.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(next)) if next.is_ascii_punctuation() => {}
            _ => out.push(c),
        }
    }
    fold(&out)
}

/// Field HTML as Markdown in a deck file of `deck`, which media links are relative to.
fn markdown(html: &str, deck: &str) -> String {
    let prefix = format!("{}{MEDIA_DIR}/", "../".repeat(deck.matches('/').count() + 1));
    text::html_to_markdown(html, &mut Media { prefix, ..Media::default() })
}

/// A note checked and ready to add.
enum Prepared {
    Note(NewNote),
    Card(NewCard),
}

fn param<'v>(params: &'v Value, name: &str) -> Result<&'v Value, String> {
    params.get(name).ok_or_else(|| format!("missing parameter: {name}"))
}

fn str_param<'v>(params: &'v Value, name: &str) -> Result<&'v str, String> {
    param(params, name)?.as_str().ok_or_else(|| format!("{name} must be text"))
}

fn array_param<'v>(params: &'v Value, name: &str) -> Result<&'v Vec<Value>, String> {
    param(params, name)?.as_array().ok_or_else(|| format!("{name} must be a list"))
}

fn ids_param(params: &Value, name: &str) -> Result<Vec<i64>, String> {
    array_param(params, name)?
        .iter()
        .map(|id| id.as_i64().ok_or_else(|| format!("{name} must be a list of ids")))
        .collect()
}

fn bool_option(value: Option<&Value>, name: &str) -> bool {
    value.and_then(|v| v.get(name)).and_then(Value::as_bool).unwrap_or(false)
}

impl Api<'_> {
    fn request(&mut self, request: &Value) -> Value {
        let version = request.get("version").and_then(Value::as_u64).unwrap_or(4);
        let result = match request.get("action").and_then(Value::as_str) {
            Some(action) => {
                let empty = json!({});
                self.action(action, request.get("params").unwrap_or(&empty))
            }
            None => Err("missing action".into()),
        };
        reply(version, result)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            cards: self.collection.cards(StudyScope::All),
            types: self.collection.note_types(),
            day_start: self.collection.day_start(),
        }
    }

    fn action(&mut self, action: &str, params: &Value) -> Result<Value, String> {
        match action {
            "version" => Ok(json!(VERSION)),
            "requestPermission" => Ok(json!({ "permission": "granted", "requireApiKey": false, "version": VERSION })),
            "apiReflect" => {
                let scopes = array_param(params, "scopes")?;
                let wanted = params.get("actions").and_then(Value::as_array);
                let actions: Vec<&str> = ACTIONS
                    .iter()
                    .copied()
                    .filter(|a| wanted.is_none_or(|w| w.iter().any(|v| v.as_str() == Some(a))))
                    .collect();
                let reflect_actions = scopes.iter().any(|s| s.as_str() == Some("actions"));
                Ok(if reflect_actions {
                    json!({ "scopes": ["actions"], "actions": actions })
                } else {
                    json!({ "scopes": [] })
                })
            }
            "multi" => {
                let actions = array_param(params, "actions")?;
                Ok(Value::Array(actions.iter().map(|request| self.request(request)).collect()))
            }
            // Files are the collection; there's nothing to sync.
            "sync" => Ok(Value::Null),

            "deckNames" => Ok(json!(self.deck_names()?)),
            "deckNamesAndIds" => {
                let decks: Map<String, Value> =
                    self.deck_names()?.into_iter().map(|d| (d.clone(), json!(name_id(&d)))).collect();
                Ok(Value::Object(decks))
            }
            "modelNames" => Ok(json!(self.snapshot().model_names())),
            "modelNamesAndIds" => {
                let models: Map<String, Value> =
                    self.snapshot().model_names().into_iter().map(|m| (m.clone(), json!(name_id(&m)))).collect();
                Ok(Value::Object(models))
            }
            "modelFieldNames" => Ok(json!(self.snapshot().model(str_param(params, "modelName")?)?.1)),
            "getTags" => Ok(json!(self.collection.tags().into_iter().map(|t| t.name).collect::<Vec<_>>())),

            "findNotes" => {
                let snapshot = self.snapshot();
                let found = find(&snapshot, str_param(params, "query")?)?;
                Ok(json!(unique(found.iter().map(|e| e.note_id))))
            }
            "findCards" => {
                let snapshot = self.snapshot();
                let found = find(&snapshot, str_param(params, "query")?)?;
                Ok(json!(found.iter().map(|e| e.card_id).collect::<Vec<_>>()))
            }
            "notesInfo" => {
                let snapshot = self.snapshot();
                let entries = snapshot.entries();
                let ids = match params.get("query").and_then(Value::as_str) {
                    Some(query) => unique(find(&snapshot, query)?.iter().map(|e| e.note_id)),
                    None => ids_param(params, "notes")?,
                };
                Ok(Value::Array(ids.iter().map(|id| note_info(&entries, *id)).collect()))
            }
            "cardsInfo" => {
                let snapshot = self.snapshot();
                let entries = snapshot.entries();
                let ids = ids_param(params, "cards")?;
                Ok(Value::Array(ids.iter().map(|id| card_info(&entries, *id)).collect()))
            }
            "areSuspended" => {
                let snapshot = self.snapshot();
                let entries = snapshot.entries();
                let ids = ids_param(params, "cards")?;
                Ok(json!(ids.iter().map(|id| entries.iter().find(|e| e.card_id == *id).map(|e| e.card.suspended)).collect::<Vec<_>>()))
            }
            "suspend" | "unsuspend" => {
                let snapshot = self.snapshot();
                let entries = snapshot.entries();
                let ids = ids_param(params, "cards")?;
                let cards: Vec<String> = entries.iter().filter(|e| ids.contains(&e.card_id)).map(|e| e.card.id.clone()).collect();
                let changed = self.collection.set_suspended(cards, action == "suspend").map_err(|e| e.to_string())?;
                self.changed |= changed > 0;
                Ok(json!(changed > 0))
            }

            "canAddNotes" | "canAddNotesWithErrorDetail" => {
                let snapshot = self.snapshot();
                let entries = snapshot.entries();
                let checks = array_param(params, "notes")?.iter().map(|note| self.check(&snapshot, &entries, note));
                Ok(if action == "canAddNotes" {
                    json!(checks.map(|c| c.is_ok()).collect::<Vec<_>>())
                } else {
                    Value::Array(
                        checks
                            .map(|c| match c {
                                Ok(()) => json!({ "canAdd": true }),
                                Err(error) => json!({ "canAdd": false, "error": error }),
                            })
                            .collect(),
                    )
                })
            }
            "addNote" => Ok(json!(self.add(param(params, "note")?)?)),
            "addNotes" => {
                let notes = array_param(params, "notes")?;
                Ok(json!(notes.iter().map(|note| self.add(note).ok()).collect::<Vec<_>>()))
            }

            "storeMediaFile" => Ok(json!(self.store_media(params)?)),
            "retrieveMediaFile" => {
                let name = media_name(str_param(params, "filename")?)?;
                match fs::read(self.collection.root().join(MEDIA_DIR).join(name)) {
                    Ok(bytes) => Ok(json!(BASE64.encode(bytes))),
                    Err(_) => Ok(json!(false)),
                }
            }
            "getMediaDirPath" => Ok(json!(self.collection.root().join(MEDIA_DIR).to_string_lossy())),

            // amgi has no browser of its own for a search, so the first note found is
            // shown in its file, as Edit Card does.
            "guiBrowse" => {
                let snapshot = self.snapshot();
                let found = find(&snapshot, str_param(params, "query")?)?;
                if let Some(first) = found.first() {
                    self.delegate.show(first.card.path.clone());
                }
                Ok(json!(found.iter().map(|e| e.card_id).collect::<Vec<_>>()))
            }
            "guiEditNote" => {
                let id = param(params, "note")?.as_i64().ok_or("note must be an id")?;
                let snapshot = self.snapshot();
                let entries = snapshot.entries();
                let entry = entries.iter().find(|e| e.note_id == id).ok_or("note was not found")?;
                self.delegate.show(entry.card.path.clone());
                Ok(Value::Null)
            }

            _ => Err("unsupported action".into()),
        }
    }

    /// The decks, with their parents and the files that have no cards yet, so a new
    /// deck can be chosen. Parents come before the decks inside them.
    fn deck_names(&self) -> Result<Vec<String>, String> {
        let decks = self.collection.decks().map_err(|e| e.to_string())?;
        let mut names: BTreeSet<Vec<String>> = decks.into_iter().map(|d| d.name.split('/').map(String::from).collect()).collect();
        let dir = self.collection.root().join(CARDS_DIR);
        let mut files = Vec::new();
        // Without a `cards` folder there are no decks.
        let _ = crate::collect_markdown(&dir, &mut files);
        for file in files {
            let Ok(deck) = file.with_extension("").strip_prefix(&dir).map(Path::to_path_buf) else { continue };
            let parts: Vec<String> = deck.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            for depth in 1..=parts.len() {
                names.insert(parts[..depth].to_vec());
            }
        }
        Ok(names.into_iter().map(|parts| parts.join("::")).collect())
    }

    /// Why `note` can't be added, if it can't.
    fn check(&self, snapshot: &Snapshot, entries: &[Entry], note: &Value) -> Result<(), String> {
        let checked = match self.prepare(snapshot, entries, note, false)? {
            Prepared::Note(note) => self.collection.check_note(&note),
            Prepared::Card(card) => self.collection.check_card(&card),
        };
        checked.map_err(|e| e.to_string())
    }

    /// Add a note, returning its id.
    fn add(&mut self, note: &Value) -> Result<i64, String> {
        let snapshot = self.snapshot();
        let entries = snapshot.entries();
        let added = match self.prepare(&snapshot, &entries, note, true)? {
            Prepared::Note(note) => self.collection.add_note(note),
            Prepared::Card(card) => self.collection.add_card(card),
        };
        let ids = added.map_err(|e| e.to_string())?;
        self.changed = true;
        Ok(ids.first().map_or(0, |id| number(id)))
    }

    /// Turn an AnkiConnect note into an amgi one. Media attached to it is only saved
    /// when `save_media` is set, as it is when the note is being added.
    fn prepare(&self, snapshot: &Snapshot, entries: &[Entry], note: &Value, save_media: bool) -> Result<Prepared, String> {
        let deck_name = str_param(note, "deckName")?.trim();
        let deck = deck_name.split("::").map(str::trim).collect::<Vec<_>>().join("/");
        let model_name = str_param(note, "modelName")?;
        let (model, names) = snapshot.model(model_name)?;
        let position = |name: &str| names.iter().position(|n| n.eq_ignore_ascii_case(name));

        // Fields are matched by name regardless of case; others are left out, as in AnkiConnect.
        let mut html = vec![String::new(); names.len()];
        let fields = param(note, "fields")?.as_object().ok_or("fields must be an object")?;
        for (name, value) in fields {
            let value = value.as_str().ok_or_else(|| format!("field {name} must be text"))?;
            if let Some(i) = position(name) {
                html[i] = value.to_string();
            }
        }
        if save_media {
            for (kind, image) in [("picture", true), ("audio", false), ("video", false)] {
                let items = match note.get(kind) {
                    Some(Value::Array(items)) => items.clone(),
                    Some(item @ Value::Object(_)) => vec![item.clone()],
                    _ => Vec::new(),
                };
                for item in &items {
                    let file = self.store_media(item)?;
                    let fields = item.get("fields").and_then(Value::as_array).cloned().unwrap_or_default();
                    for i in fields.iter().filter_map(Value::as_str).filter_map(&position) {
                        html[i].push_str(&if image { format!(r#"<img src="{file}">"#) } else { format!("[sound:{file}]") });
                    }
                }
            }
        }
        let values: Vec<String> = html.iter().map(|h| markdown(h, &deck)).collect();
        let first = values.first().map(|v| plain(v)).unwrap_or_default();
        if first.trim().is_empty() {
            return Err("cannot create note because it is empty".into());
        }

        let options = note.get("options");
        if !bool_option(options, "allowDuplicate") {
            let scope = options.and_then(|o| o.get("duplicateScopeOptions"));
            let same_deck = options.and_then(|o| o.get("duplicateScope")).and_then(Value::as_str) == Some("deck");
            let scope_deck = scope.and_then(|s| s.get("deckName")).and_then(Value::as_str).unwrap_or(deck_name);
            let scope_deck = fold(scope_deck);
            let children = bool_option(scope, "checkChildren");
            let all_models = bool_option(scope, "checkAllModels");
            let duplicate = entries.iter().any(|e| {
                let deck = fold(&e.deck);
                (all_models || e.model == model_name)
                    && (!same_deck || deck == scope_deck || (children && deck.starts_with(&format!("{scope_deck}::"))))
                    && e.plain.first() == Some(&first)
            });
            if duplicate {
                return Err("cannot create note because it is a duplicate".into());
            }
        }

        let tags = match note.get("tags") {
            Some(Value::Array(tags)) => tags.iter().filter_map(Value::as_str).map(String::from).collect(),
            _ => Vec::new(),
        };
        Ok(match model {
            Model::NoteType => Prepared::Note(NewNote { deck, note_type: model_name.into(), values, tags }),
            Model::Basic => Prepared::Card(NewCard {
                deck,
                kind: CardKind::Basic,
                front: values[0].clone(),
                back: values[1].clone(),
                tags,
            }),
            Model::Cloze => {
                if !values[1].is_empty() {
                    return Err("amgi's clozes have no Back Extra; leave it empty".into());
                }
                let (cloze, _) = text::convert_clozes(&values[0], |_| true);
                Prepared::Card(NewCard { deck, kind: CardKind::Cloze, front: cloze, back: String::new(), tags })
            }
        })
    }

    /// Save a media file given as base64 `data` or a local `path`, returning the name
    /// it was saved under. A different file with the same name is kept, and this one
    /// named anew.
    fn store_media(&self, params: &Value) -> Result<String, String> {
        let name = media_name(str_param(params, "filename")?)?;
        let bytes = if let Some(data) = params.get("data").and_then(Value::as_str) {
            BASE64.decode(data.trim()).map_err(|e| format!("invalid base64 data: {e}"))?
        } else if let Some(path) = params.get("path").and_then(Value::as_str) {
            fs::read(path).map_err(|e| format!("can't read {path}: {e}"))?
        } else if params.get("url").is_some() {
            return Err("amgi can't download media; send the file's data instead".into());
        } else {
            return Err("media needs data or a path".into());
        };
        self.collection.save_media(name, &bytes).map_err(|e| e.to_string())
    }
}

/// A media file name, which must stay inside the flat `media` folder.
fn media_name(name: &str) -> Result<&str, String> {
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\', '\0']) {
        return Err(format!("invalid media file name: {name}"));
    }
    Ok(name)
}

/// A stable id for a deck or model name.
fn name_id(name: &str) -> i64 {
    (crate::parser::stable_hash(&[name]) >> 11) as i64
}

fn unique(ids: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut seen = HashSet::new();
    ids.filter(|id| seen.insert(*id)).collect()
}

/// The cards matching an Anki search. An empty search finds nothing.
fn find<'s>(snapshot: &'s Snapshot, query: &str) -> Result<Vec<Entry<'s>>, String> {
    let Some(node) = query::parse(query)? else { return Ok(Vec::new()) };
    Ok(snapshot.entries().into_iter().filter(|e| node.matches(e, snapshot.day_start)).collect())
}

fn fields_json(entry: &Entry) -> Value {
    let fields: Map<String, Value> = entry
        .fields
        .iter()
        .enumerate()
        .map(|(order, (name, value))| (name.clone(), json!({ "value": value, "order": order })))
        .collect();
    Value::Object(fields)
}

/// A note as `notesInfo` describes it; `{}` when there's no such note.
fn note_info(entries: &[Entry], id: i64) -> Value {
    let cards: Vec<&Entry> = entries.iter().filter(|e| e.note_id == id).collect();
    let Some(first) = cards.first() else { return json!({}) };
    json!({
        "noteId": id,
        "profile": "",
        "modelName": first.model,
        "tags": first.card.tags,
        "fields": fields_json(first),
        "cards": cards.iter().map(|e| e.card_id).collect::<Vec<_>>(),
        "mod": 0,
    })
}

/// A card as `cardsInfo` describes it; `{}` when there's no such card.
fn card_info(entries: &[Entry], id: i64) -> Value {
    let Some(entry) = entries.iter().find(|e| e.card_id == id) else { return json!({}) };
    let card = entry.card;
    // Anki's card types: 0 new, 1 learning, 2 review; its queue is -1 when suspended.
    let kind = match card.interval {
        None => 0,
        Some(i) if i < crate::DAY_SECS => 1,
        Some(_) => 2,
    };
    json!({
        "cardId": id,
        "note": entry.note_id,
        "deckName": entry.deck,
        "modelName": entry.model,
        "fieldOrder": 0,
        "fields": fields_json(entry),
        "question": card.front,
        "answer": card.back,
        "ord": entry.ord,
        "type": kind,
        "queue": if card.suspended { -1 } else { kind },
        "due": card.due.map_or(0, |due| due / crate::DAY_SECS),
        "interval": card.interval.map_or(0, |i| i / crate::DAY_SECS),
        "reps": card.reps,
        "lapses": card.lapses,
        "left": 0,
        "mod": 0,
        "flags": 0,
    })
}

#[cfg(test)]
mod tests;
