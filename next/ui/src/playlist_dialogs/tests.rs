use std::cell::RefCell;
use std::rc::Rc;

use anti_webui::core::Ui;
use anti_webui::core::access::snapshot;
use anti_webui::core::event::NamedKey;
use anti_webui::core::tree::NodeKind;
use hiphile_core::test_support::PlaylistsServer;
use serde_json::json;

use super::*;
use crate::testing::{
    TestCore, find, frames, key, nodes, press, recording_commands, screen_context, signed_in_core,
    start, typed, wait_until,
};

fn songs(ids: &[&str]) -> Arc<[Song]> {
    ids.iter()
        .map(|id| {
            serde_json::from_value(json!({ "id": id, "title": format!("Song {id}") })).unwrap()
        })
        .collect()
}

struct Shown {
    ui: Ui,
    dialogs: PlaylistDialogs,
    sent: Rc<RefCell<Vec<Command>>>,
    server: PlaylistsServer,
    _core: TestCore,
}

/// The playlist dialogs of a window on a server with `playlists`, once their choices are known.
fn on(playlists: &[(&str, &str, &[&str])]) -> Shown {
    let server = PlaylistsServer::new(playlists);
    let (core, hiphile) = signed_in_core(server.url());
    let (commands, sent) = recording_commands();
    let dialogs = PlaylistDialogs::new();
    let made = Rc::new(RefCell::new(None));
    let mut ui = start({
        let made = made.clone();
        move || {
            let cx = screen_context(&hiphile, commands);
            made.replace(Some(cx.clone()));
            div().child(dialogs.view(&cx)).child(cx.snackbars.view())
        }
    });
    let cx: ScreenContext = made.take().unwrap();
    let known = cx.known;
    let count = playlists.len();
    wait_until(&mut ui, || known.playlist_choices().len() == count);
    Shown {
        ui,
        dialogs,
        sent,
        server,
        _core: core,
    }
}

impl Shown {
    fn open(&mut self, dialog: PlaylistDialog) {
        self.dialogs.open(dialog);
        frames(&mut self.ui, 400);
    }

    fn is_closed(&mut self) -> bool {
        frames(&mut self.ui, 400);
        nodes(Role::Dialog).is_empty() && !self.dialogs.is_open()
    }
}

fn shows_text_with(part: &str) -> bool {
    with_doc(|doc| {
        doc.nodes.iter().any(
            |(_, node)| matches!(&node.kind, NodeKind::Text(shown) if shown.text.contains(part)),
        )
    })
}

fn focused() -> Option<anti_webui::core::access::AccessNode> {
    let tree = snapshot();
    let focus = tree.focus?;
    tree.nodes.into_iter().find(|node| node.id == focus)
}

#[test]
fn a_new_playlist_is_named_and_made_with_the_songs() {
    let mut shown = on(&[]);
    shown.open(PlaylistDialog::New(songs(&["s1"])));
    assert!(find(Role::Heading, "New Playlist").is_some());
    assert!(shows_text_with("Starts with “Song s1”"));
    assert_eq!(
        focused().and_then(|node| node.label).as_deref(),
        Some("Name")
    );

    typed(&mut shown.ui, "Mix");
    press(&mut shown.ui, Role::Button, "Create");
    let server = &shown.server;
    wait_until(&mut shown.ui, || {
        !server.requests("createPlaylist").is_empty() && !nodes(Role::Status).is_empty()
    });

    let created = &server.requests("createPlaylist")[0];
    assert_eq!(created.values("name"), ["Mix"]);
    assert_eq!(created.values("songId"), ["s1"]);
    assert!(shown.is_closed());
    assert!(shown.sent.borrow().is_empty());
}

#[test]
fn an_empty_new_playlist_is_shown_once_made() {
    let mut shown = on(&[]);
    shown.open(PlaylistDialog::New(songs(&[])));
    assert!(!shows_text_with("Starts with"));

    typed(&mut shown.ui, "Later");
    key(&mut shown.ui, NamedKey::Enter);
    let sent = shown.sent.clone();
    wait_until(&mut shown.ui, || !sent.borrow().is_empty());

    assert_eq!(
        *sent.borrow(),
        [Command::Show(Location::Playlists(Some(PlaylistId::new(
            "new1"
        ))))]
    );
    assert!(shown.is_closed());
}

#[test]
fn a_blank_name_makes_nothing() {
    let mut shown = on(&[]);
    shown.open(PlaylistDialog::New(songs(&["s1"])));
    assert!(find(Role::Button, "Create").unwrap().disabled);

    typed(&mut shown.ui, "   ");
    key(&mut shown.ui, NamedKey::Enter);

    assert!(find(Role::Button, "Create").unwrap().disabled);
    assert!(shown.server.requests("createPlaylist").is_empty());
    assert!(!shown.is_closed());
}

#[test]
fn a_playlist_that_could_not_be_made_leaves_the_dialog_open_with_why() {
    let mut shown = on(&[]);
    shown.server.refuse("createPlaylist");
    shown.open(PlaylistDialog::New(songs(&["s1"])));

    typed(&mut shown.ui, "Road Trip");
    press(&mut shown.ui, Role::Button, "Create");
    wait_until(&mut shown.ui, || shows_text_with("Refused"));

    assert!(!shown.is_closed());
    assert!(!find(Role::Button, "Create").unwrap().disabled);

    typed(&mut shown.ui, "!");
    assert!(!shows_text_with("Refused"));
}

#[test]
fn it_cannot_be_closed_while_the_playlist_is_being_made() {
    let mut shown = on(&[]);
    shown.server.hold("createPlaylist");
    shown.open(PlaylistDialog::New(songs(&["s1"])));
    typed(&mut shown.ui, "Road Trip");
    press(&mut shown.ui, Role::Button, "Create");
    let server = &shown.server;
    wait_until(&mut shown.ui, || {
        !server.requests("createPlaylist").is_empty()
    });

    key(&mut shown.ui, NamedKey::Escape);
    assert!(!shown.is_closed());
    assert!(find(Role::Button, "Cancel").unwrap().disabled);

    shown.server.release("createPlaylist");
    let ui = &mut shown.ui;
    wait_until(ui, || !nodes(Role::Status).is_empty());
    assert!(shown.is_closed());
}

#[test]
fn escape_closes_it_otherwise() {
    let mut shown = on(&[]);
    shown.open(PlaylistDialog::New(songs(&["s1"])));

    key(&mut shown.ui, NamedKey::Escape);

    assert!(shown.is_closed());
}

#[test]
fn one_open_is_not_gone_over() {
    let mut shown = on(&[("p1", "Road Trip", &[])]);
    shown.open(PlaylistDialog::Choose(songs(&["s1"])));

    shown.open(PlaylistDialog::New(songs(&["s2"])));

    assert!(find(Role::Heading, "Add to Playlist").is_some());
    assert!(find(Role::Heading, "New Playlist").is_none());
}

#[test]
fn the_playlist_chosen_with_the_keys_gets_the_songs() {
    let mut shown = on(&[("p1", "Focus", &[]), ("p2", "Road Trip", &[])]);
    shown.open(PlaylistDialog::Choose(songs(&["s1"])));
    assert!(find(Role::Heading, "Add to Playlist").is_some());
    assert_eq!(
        focused().and_then(|node| node.label).as_deref(),
        Some("Playlists")
    );

    key(&mut shown.ui, NamedKey::ArrowDown);
    key(&mut shown.ui, NamedKey::Enter);

    assert_eq!(
        *shown.sent.borrow(),
        [Command::AddToPlaylist {
            playlist: PlaylistId::new("p2"),
            songs: songs(&["s1"]),
        }]
    );
    assert!(shown.is_closed());
}

#[test]
fn add_takes_the_songs_to_the_first_playlist_unless_another_is_chosen() {
    let mut shown = on(&[("p1", "Focus", &[]), ("p2", "Road Trip", &[])]);
    shown.open(PlaylistDialog::Choose(songs(&["s1"])));

    press(&mut shown.ui, Role::Button, "Add");

    assert_eq!(
        *shown.sent.borrow(),
        [Command::AddToPlaylist {
            playlist: PlaylistId::new("p1"),
            songs: songs(&["s1"]),
        }]
    );
}

#[test]
fn new_playlist_names_one_for_the_songs_instead() {
    let mut shown = on(&[("p1", "Road Trip", &[])]);
    shown.open(PlaylistDialog::Choose(songs(&["s1", "s2"])));

    press(&mut shown.ui, Role::Button, "New Playlist…");
    frames(&mut shown.ui, 400);

    assert!(find(Role::Heading, "New Playlist").is_some());
    assert!(shows_text_with("Starts with 2 songs"));
}
