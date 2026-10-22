use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::server::AnkiConnectServer;
use super::*;

#[derive(Default)]
struct Recorder {
    changes: Mutex<u32>,
    shown: Mutex<Vec<String>>,
}

impl AnkiConnectDelegate for Recorder {
    fn cards_changed(&self) {
        *self.changes.lock().unwrap() += 1;
    }
    fn show(&self, path: String) {
        self.shown.lock().unwrap().push(path);
    }
}

fn collection(files: &[(&str, &str)]) -> (PathBuf, Arc<CardCollection>) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("amgi-ankiconnect-test-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (name, body) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let c = CardCollection::open(dir.to_string_lossy().into_owned()).unwrap();
    (dir, c)
}

/// Send a request and parse the reply.
fn call(c: &CardCollection, recorder: &Recorder, request: Value) -> Value {
    serde_json::from_str(&handle(c, &request.to_string(), recorder)).unwrap()
}

const VOCAB: &str = "fields = [\"Word\", \"Reading\", \"Meaning\", \"Audio\"]\ncards = [\"Word -> Reading, Meaning, Audio\"]\n";

#[test]
fn yomitan_flow() {
    let (dir, c) = collection(&[("note-types/Vocab.toml", VOCAB), ("cards/lang/ja.md", "+++\nname = \"Japanese\"\n+++\n")]);
    let r = Recorder::default();
    // Yomitan sends version 2: results come back bare.
    let v2 = |action: &str, params: Value| call(&c, &r, json!({ "action": action, "params": params, "version": 2 }));

    assert_eq!(v2("version", json!({})), json!(6));
    assert_eq!(v2("deckNames", json!({})), json!(["lang", "lang::ja"]));
    assert_eq!(v2("modelNames", json!({})), json!(["Vocab", "Basic", "Cloze"]));
    assert_eq!(v2("modelFieldNames", json!({ "modelName": "Vocab" })), json!(["Word", "Reading", "Meaning", "Audio"]));
    assert_eq!(v2("modelFieldNames", json!({ "modelName": "Nope" })), json!({ "result": null, "error": "model was not found: Nope" }));

    let note = json!({
        "deckName": "lang::ja",
        "modelName": "Vocab",
        "fields": {
            "word": "日本",
            "Reading": "<ruby>日本<rt>にほん</rt></ruby>",
            "Meaning": "<div><ol><li>Japan</li><li>a <b>country</b></li></ol></div>",
            "Audio": "[sound:nihon.mp3]",
        },
        "tags": ["yomitan"],
        "options": { "allowDuplicate": false, "duplicateScope": "collection" },
    });
    assert_eq!(v2("canAddNotesWithErrorDetail", json!({ "notes": [note] })), json!([{ "canAdd": true }]));
    assert_eq!(v2("storeMediaFile", json!({ "filename": "nihon.mp3", "data": "c291bmQ=" })), json!("nihon.mp3"));
    assert_eq!(fs::read(dir.join("media/nihon.mp3")).unwrap(), b"sound");
    // Nothing's added until then.
    assert_eq!(*r.changes.lock().unwrap(), 0);

    let id = v2("addNote", json!({ "note": note }));
    assert!(id.as_i64().unwrap() > 0);
    assert_eq!(*r.changes.lock().unwrap(), 1);
    assert_eq!(
        fs::read_to_string(dir.join("cards/lang/ja.md")).unwrap(),
        "+++\nname = \"Japanese\"\n+++\n\n@Vocab\nWord: 日本\nReading: 日本（にほん）\nMeaning: - Japan\n- a **country**\nAudio: ![](../../media/nihon.mp3)\n#yomitan\n"
    );

    // Yomitan's duplicate search, alone and as it batches several.
    assert_eq!(v2("findNotes", json!({ "query": "\"deck:lang::ja\" \"word:日本\"" })), json!([id]));
    assert_eq!(v2("findNotes", json!({ "query": "\"word:日\"" })), json!([]));
    let union = v2(
        "multi",
        json!({ "actions": [
            { "action": "findNotes", "params": { "query": format!("nid:{id} (\"word:日本\")") } },
            { "action": "findNotes", "params": { "query": "(\"word:日本\") or (\"word:中国\")" } },
            { "action": "findNotes", "params": { "query": "deck:lang -tag:yomitan" } },
        ] }),
    );
    assert_eq!(union, json!([[id], [id], []]));
    assert_eq!(
        v2("canAddNotesWithErrorDetail", json!({ "notes": [note] })),
        json!([{ "canAdd": false, "error": "cannot create note because it is a duplicate" }])
    );
    // In another deck, a duplicate only counts when the whole collection is checked.
    let mut elsewhere = note.clone();
    elsewhere["deckName"] = json!("other");
    elsewhere["options"]["duplicateScope"] = json!("deck");
    assert_eq!(v2("canAddNotes", json!({ "notes": [elsewhere] })), json!([false]));
    elsewhere["options"]["allowDuplicate"] = json!(true);
    // amgi still won't have two cards with one front.
    let detail = v2("canAddNotesWithErrorDetail", json!({ "notes": [elsewhere] }));
    assert!(detail[0]["error"].as_str().unwrap().contains("already in cards/lang/ja.md:5"));

    let info = v2("notesInfo", json!({ "notes": [id, 1] }));
    assert_eq!(info[0]["modelName"], "Vocab");
    assert_eq!(info[0]["fields"]["Reading"], json!({ "value": "日本（にほん）", "order": 1 }));
    assert_eq!(info[0]["tags"], json!(["yomitan"]));
    assert_eq!(info[1], json!({}));
    let card = info[0]["cards"][0].clone();
    let cards = v2("cardsInfo", json!({ "cards": [card] }));
    assert_eq!((cards[0]["note"].clone(), cards[0]["queue"].clone(), cards[0]["deckName"].clone()), (json!(id), json!(0), json!("lang::ja")));

    assert_eq!(v2("suspend", json!({ "cards": [card] })), json!(true));
    assert!(fs::read_to_string(dir.join("cards/lang/ja.md")).unwrap().contains("#yomitan #suspended"));
    assert_eq!(v2("areSuspended", json!({ "cards": [card, 1] })), json!([true, null]));
    assert_eq!(v2("findCards", json!({ "query": "is:suspended" })), json!([card]));

    assert_eq!(v2("guiEditNote", json!({ "note": id })), json!(null));
    assert_eq!(*r.shown.lock().unwrap(), ["cards/lang/ja.md"]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn basic_and_cloze() {
    let (dir, c) = collection(&[]);
    let r = Recorder::default();
    let add = |model: &str, fields: Value| {
        call(&c, &r, json!({ "action": "addNote", "version": 6, "params": { "note": {
            "deckName": "Geography", "modelName": model, "fields": fields,
            "picture": [{ "filename": "map.png", "data": "cG5n", "fields": ["Back"] }],
        } } }))
    };
    let basic = add("Basic", json!({ "Front": "Capital of France?", "Back": "Paris" }));
    assert!(basic["result"].is_i64(), "{basic}");
    let cloze = add("Cloze", json!({ "Text": "The capital of {{c1::Italy}} is {{c2::Rome::city}}." }));
    assert!(cloze["result"].is_i64(), "{cloze}");
    assert_eq!(
        fs::read_to_string(dir.join("cards/Geography.md")).unwrap(),
        "Q: Capital of France?\nA: Paris![](../media/map.png)\n\nC: The capital of [Italy] is [Rome|city].\n"
    );
    assert_eq!(
        add("Cloze", json!({ "Text": "x", "Back Extra": "y" })),
        json!({ "result": null, "error": "amgi's clozes have no Back Extra; leave it empty" })
    );
    assert_eq!(add("Basic", json!({ "Back": "x" }))["error"], "cannot create note because it is empty");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn errors_and_versions() {
    let (dir, c) = collection(&[]);
    let r = Recorder::default();
    assert_eq!(call(&c, &r, json!({ "action": "version", "version": 6 })), json!({ "result": 6, "error": null }));
    assert_eq!(call(&c, &r, json!({ "action": "nope", "version": 6 })), json!({ "result": null, "error": "unsupported action" }));
    assert_eq!(call(&c, &r, json!({ "action": "findNotes", "params": { "query": "added:1" } }))["error"], "amgi can't search added:");
    assert_eq!(
        call(&c, &r, json!({ "action": "storeMediaFile", "params": { "filename": "../x", "data": "" } }))["error"],
        "invalid media file name: ../x"
    );
    assert_eq!(
        call(&c, &r, json!({ "action": "apiReflect", "params": { "scopes": ["actions"], "actions": ["addNote", "updateNoteFields"] } })),
        json!({ "scopes": ["actions"], "actions": ["addNote"] })
    );
    assert_eq!(handle(&c, "not json", &r), r#"{"error":"invalid request: expected ident at line 1 column 2","result":null}"#);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn server_round_trip() {
    let (dir, c) = collection(&[]);
    let server = AnkiConnectServer::start(c, 0, Arc::new(Recorder::default())).unwrap();
    let send = |origin: &str| {
        let body = r#"{"action":"version","version":6}"#;
        let mut stream = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
        write!(stream, "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: {origin}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply
    };
    let ok = send("chrome-extension://abc");
    assert!(ok.starts_with("HTTP/1.1 200 OK\r\n"), "{ok}");
    assert!(ok.contains("Access-Control-Allow-Origin: chrome-extension://abc\r\n"));
    assert!(ok.ends_with(r#"{"error":null,"result":6}"#));
    assert!(send("https://example.com").starts_with("HTTP/1.1 403 Forbidden"));

    let port = server.port();
    server.stop();
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    fs::remove_dir_all(dir).unwrap();
}
