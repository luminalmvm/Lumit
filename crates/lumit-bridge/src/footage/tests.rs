use super::*;
use lumit_core::model::MediaRef;
use lumit_core::ops::Op;
use lumit_media::encode::{ColourTags, Encoder, Metadata, VideoCodec, VideoSettings};
use std::net::{IpAddr, Ipv4Addr};

fn until(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(60), "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A clip one person has and the other has not is read by the other
/// from a stand-in the first makes when asked: a file of the clip's own
/// size with every one of its frames. Asked for the original, it is
/// the file itself that arrives, and the stand-in gives way to it.
#[test]
fn a_clip_missing_here_is_read_from_what_another_person_sends() {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("clip.mp4");
    let settings = VideoSettings {
        codec: VideoCodec::H264,
        width: 320,
        height: 240,
        fps_num: 30,
        fps_den: 1,
        bit_rate: None,
        max_rate: None,
        cbr: false,
        colour: ColourTags::default(),
        depth: lumit_media::encode::BitDepth::Eight,
    };
    let mut encoder = Encoder::open(&clip, Some(&settings), None, &Metadata::new()).unwrap();
    for n in 0..12u8 {
        let frame: Vec<u8> = [n * 20, 90, 200, 255].repeat(320 * 240);
        encoder.write_rgba(&frame).unwrap();
    }
    encoder.finish().unwrap();

    // The host has the clip in its project.
    let item = Uuid::now_v7();
    let value = json!({ "id": item, "name": "clip", "media": { "relative_path": "clip.mp4" } });
    let mut clip_item: FootageItem = serde_json::from_value(value).unwrap();
    clip_item.media = MediaRef {
        absolute_path: clip.to_string_lossy().into_owned(),
        fingerprint: lumit_project::fingerprint_path(&clip).ok(),
        ..clip_item.media
    };
    let hosted = Arc::new(DocumentStore::new(Document::new()));
    let added = Op::AddItem {
        index: 0,
        item: Box::new(ProjectItem::Footage(clip_item)),
    };
    hosted.commit(added).unwrap();

    // Two machines in one process: each with a project, an end of the
    // sharing, and a folder of its own for what it is sent.
    let quiet: lumit_share::Events = Arc::new(|_| {});
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let host = lumit_share::host::host(
        hosted.clone(),
        "Host",
        loopback,
        0,
        None,
        None,
        None,
        quiet.clone(),
    )
    .unwrap();
    let (document, joining) =
        lumit_share::guest::join(host.invite("127.0.0.1"), "Guest", None).unwrap();
    let joined = Arc::new(DocumentStore::new(document));
    let guest = joining.start(joined.clone(), quiet).unwrap();
    let (host_project, guest_project) = (Uuid::now_v7(), Uuid::now_v7());
    let tell: Tell = Arc::new(|_| {});
    let machine = |project: Uuid, store: &Arc<DocumentStore>, sharing, name: &str| {
        crate::api::share::share_for_test(project, sharing);
        let root = dir.path().join(name).join(SENT);
        let carrier = Carrier::start_under(project, store.clone(), tell.clone(), Some(root));
        crate::api::share::with_sharing(project, |sharing| {
            sharing.carry_footage(carrier.clone(), LIMITS.clone());
        });
        carrier
    };
    let _hosts = machine(
        host_project,
        &hosted,
        lumit_share::Sharing::Host(host),
        "host",
    );
    let guests = machine(
        guest_project,
        &joined,
        lumit_share::Sharing::Guest(guest),
        "guest",
    );
    let at_guest = || {
        let doc = joined.snapshot();
        here(footage(&doc, item).unwrap())
    };
    assert_eq!(at_guest(), Here::Missing);
    until("the guest hears who has the clip", || {
        crate::api::share::with_sharing(guest_project, |s| !s.holders(item).is_empty())
            == Some(true)
    });

    guests.fetch(item, false);
    until("a stand-in arrives and is read from", || {
        matches!(at_guest(), Here::StandIn(_))
    });
    let Here::StandIn(stand_in) = at_guest() else {
        panic!("a stand-in");
    };
    let frames = |path: &Path| {
        let source = lumit_media::sequence::MediaSource::file(path.to_path_buf());
        lumit_render::media_index::load_or_build_index(&source)
            .unwrap()
            .frame_count()
    };
    assert_eq!(frames(&stand_in), frames(&clip));
    assert!(stand_in.starts_with(dir.path().join("guest")));

    guests.fetch(item, true);
    until("the original arrives and takes over", || {
        matches!(at_guest(), Here::Original(_))
    });
    let Here::Original(original) = at_guest() else {
        panic!("the original");
    };
    assert_eq!(
        std::fs::read(original).unwrap(),
        std::fs::read(&clip).unwrap()
    );

    crate::api::share::stop(host_project);
    crate::api::share::stop(guest_project);
}

/// Two machines sharing a project that has a clip in a composition: the
/// host has the clip and the guest has not.
struct Rig {
    dir: tempfile::TempDir,
    hosted: Arc<DocumentStore>,
    joined: Arc<DocumentStore>,
    hosts: Arc<Carrier>,
    guests: Arc<Carrier>,
    projects: (Uuid, Uuid),
    item: Uuid,
    comp: Uuid,
    /// The exports the host has been asked to do.
    asked: Arc<Mutex<Vec<Uuid>>>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        crate::api::share::stop(self.projects.0);
        crate::api::share::stop(self.projects.1);
    }
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("clip.mp4");
    let settings = VideoSettings {
        codec: VideoCodec::H264,
        width: 320,
        height: 240,
        fps_num: 30,
        fps_den: 1,
        bit_rate: None,
        max_rate: None,
        cbr: false,
        colour: ColourTags::default(),
        depth: lumit_media::encode::BitDepth::Eight,
    };
    let mut encoder = Encoder::open(&clip, Some(&settings), None, &Metadata::new()).unwrap();
    for n in 0..12u8 {
        let frame: Vec<u8> = [n * 20, 90, 200, 255].repeat(320 * 240);
        encoder.write_rgba(&frame).unwrap();
    }
    encoder.finish().unwrap();

    // A project with the clip in a composition, made the way the
    // interface makes one, and then its document on its own.
    let project = crate::api::state::LumitBridgeState::new_project(None).unwrap();
    let comp = project.new_composition("Scene".into(), None).unwrap();
    let placed = project
        .import_footage(clip.to_string_lossy().into_owned())
        .unwrap();
    comp.add_footage_layer(&placed, false, None, None).unwrap();
    let (item, comp) = (placed.id, comp.id);
    let document = {
        let state = project.state().unwrap();
        let state = state.read().unwrap();
        Document::clone(&state.store.snapshot())
    };
    project.close().unwrap();

    let hosted = Arc::new(DocumentStore::new(document));
    let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let projects = (Uuid::now_v7(), Uuid::now_v7());
    // A note reaches a machine's carrier as the bridge hands it on.
    let notes_for = |project: Uuid| -> lumit_share::Events {
        Arc::new(move |event| {
            if let lumit_share::Event::Note { from, body } = event {
                if let Some(carrier) = carrier(project) {
                    carrier.note(from, body);
                }
            }
        })
    };
    let events = notes_for(projects.0);
    let host = lumit_share::host::host(
        hosted.clone(),
        "Host",
        loopback,
        0,
        None,
        None,
        None,
        events,
    )
    .unwrap();
    let (document, joining) =
        lumit_share::guest::join(host.invite("127.0.0.1"), "Guest", None).unwrap();
    let joined = Arc::new(DocumentStore::new(document));
    let guest = joining
        .start(joined.clone(), notes_for(projects.1))
        .unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let heard = asked.clone();
    let tell: Tell = Arc::new(move |told| {
        if let Told::Asked { job, .. } = told {
            heard.lock().unwrap().push(job);
        }
    });
    let machine = |project: Uuid, store: &Arc<DocumentStore>, sharing, name: &str| {
        crate::api::share::share_for_test(project, sharing);
        let root = dir.path().join(name).join(SENT);
        let carrier = Carrier::start_under(project, store.clone(), tell.clone(), Some(root));
        crate::api::share::with_sharing(project, |sharing| {
            sharing.carry_footage(carrier.clone(), LIMITS.clone());
        });
        carrier
    };
    let hosts = machine(
        projects.0,
        &hosted,
        lumit_share::Sharing::Host(host),
        "host",
    );
    let guests = machine(
        projects.1,
        &joined,
        lumit_share::Sharing::Guest(guest),
        "guest",
    );
    Rig {
        dir,
        hosted,
        joined,
        hosts,
        guests,
        projects,
        item,
        comp,
        asked,
    }
}

/// An export on a machine without the clip fetches the frames it reads
/// and is queued from them. Asking the other machine to do it instead
/// reaches that machine as a question, and its no comes back.
#[test]
fn an_export_fetches_its_frames_or_is_asked_of_whoever_has_them() {
    let _queue = crate::api::tests::export_queue_test();
    let rig = rig();
    let (item, comp) = (rig.item, rig.comp);
    // The clip is in a composition, so the guest asks for a stand-in of
    // it without being told to.
    until("a stand-in arrives by itself", || {
        let doc = rig.joined.snapshot();
        matches!(here(footage(&doc, item).unwrap()), Here::StandIn(_))
    });
    assert_eq!(lacking(&rig.joined.snapshot(), comp), [item]);
    assert!(lacking(&rig.hosted.snapshot(), comp).is_empty());

    let out = rig.dir.path().join("out.mp4");
    let out = out.to_string_lossy().into_owned();
    let spec = BridgeExportSpec::default();
    let name = String::from("Scene");
    rig.guests
        .fetch_then_export(comp, name, spec.clone(), out.clone(), true, false);
    until("the frames arrive and the export is queued", || {
        assert_ne!(rig.guests.fetching(), Fetching::Failed);
        matches!(rig.guests.fetching(), Fetching::Queued { .. })
    });
    let Fetching::Queued { id } = rig.guests.fetching() else {
        panic!("queued");
    };
    crate::export::queue_remove(id);
    let sent: Vec<String> = std::fs::read_dir(rig.dir.path().join("guest").join(SENT))
        .unwrap()
        .flatten()
        .flat_map(|folder| {
            std::fs::read_dir(folder.path())
                .into_iter()
                .flatten()
                .flatten()
        })
        .map(|file| file.file_name().to_string_lossy().into_owned())
        .collect();
    // The names say what came instead when it fails, which only CI's
    // Macs have seen.
    assert!(
        sent.iter().any(|name| name.starts_with("parts-")),
        "the frames the export reads came as a file of their own: {sent:?}"
    );

    // Asked of the host instead, the host is asked, and says no.
    rig.guests.ask_export(0, comp, &spec, &out);
    until("the host is asked", || {
        !rig.asked.lock().unwrap().is_empty()
    });
    assert_eq!(rig.guests.asking(), Asking::Waiting);
    let job = rig.asked.lock().unwrap()[0];
    rig.hosts.answer_export(job, false);
    until("the no comes back", || {
        let refused = Asking::Failed {
            why: "refused".into(),
        };
        rig.guests.asking() == refused
    });
}

/// The whole of an export done by another machine: asked, agreed to,
/// exported there from the originals, and the file sent back to where
/// the person who asked wanted it. It runs a real export, which needs a
/// GPU and takes the queue, so it is run by hand:
/// `cargo test -p lumit_bridge --lib asked_of_another -- --ignored`.
#[test]
#[ignore = "runs a real export"]
fn an_export_asked_of_another_machine_comes_back() {
    let _queue = crate::api::tests::export_queue_test();
    let rig = rig();
    let out = rig.dir.path().join("out.mp4");
    // A dozen frames: it is the journey that is under test.
    let spec = BridgeExportSpec {
        range_start_frame: 0,
        range_end_frame: 12,
        ..BridgeExportSpec::default()
    };
    rig.guests
        .ask_export(0, rig.comp, &spec, &out.to_string_lossy());
    until("the host is asked", || {
        !rig.asked.lock().unwrap().is_empty()
    });
    let job = rig.asked.lock().unwrap()[0];
    rig.hosts.answer_export(job, true);
    crate::export::queue_start();
    until("the file comes back", || {
        // The queue is turned over by being read, as the interface does.
        let _ = crate::export::queue_list();
        assert!(
            !matches!(rig.guests.asking(), Asking::Failed { .. }),
            "{:?}",
            rig.guests.asking()
        );
        matches!(rig.guests.asking(), Asking::Done { .. })
    });
    assert!(std::fs::metadata(&out).is_ok_and(|file| file.len() > 1000));
}
