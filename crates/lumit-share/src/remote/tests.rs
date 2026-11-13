use super::*;
use crate::bulk::News;
use std::path::{Path, PathBuf};

/// One machine's files for the tests: what it can give, by id, and a folder
/// that what it is sent lands in.
struct Shelf {
    folder: PathBuf,
    gives: Vec<(Uuid, PathBuf)>,
    arrived: Mutex<Vec<Wanted>>,
}

/// The item, or the job for an export.
fn id(wanted: &Wanted) -> Option<Uuid> {
    match wanted {
        Wanted::Export { job } => Some(*job),
        other => other.item(),
    }
}

impl Footage for Shelf {
    fn make(&self, wanted: &Wanted, _: &AtomicBool) -> Option<PathBuf> {
        let id = id(wanted)?;
        let given = self.gives.iter().find(|(given, _)| *given == id);
        given.map(|(_, path)| path.clone())
    }

    fn room(&self, wanted: &Wanted) -> Option<PathBuf> {
        Some(self.folder.join(format!("{}.bin", id(wanted)?)))
    }

    fn told(&self, news: News) {
        if let News::Arrived { wanted, .. } = news {
            self.arrived.lock().push(wanted);
        }
    }
}

/// A folder of the test's own under the system's temp folder, emptied first.
fn folder(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("lumit-remote-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn shelf(folder: &Path, name: &str, gives: Vec<(Uuid, PathBuf)>) -> Arc<Shelf> {
    let folder = folder.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    Arc::new(Shelf {
        folder,
        gives,
        arrived: Mutex::new(Vec::new()),
    })
}

/// `size` bytes no two runs of which line up by chance, written at `path`.
fn file(path: &Path, size: usize) -> Vec<u8> {
    let bytes: Vec<u8> = (0..size)
        .map(|n| ((n as u32).wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

type Said = Arc<Mutex<Vec<LinkEvent>>>;

fn heard() -> (Said, Box<dyn Fn(LinkEvent) + Send + Sync>) {
    let said = Said::default();
    let saying = said.clone();
    (said, Box::new(move |event| saying.lock().push(event)))
}

fn key(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn limits() -> Arc<Limits> {
    Arc::new(Limits::default())
}

/// Wait for `done`, failing the test by `secs` seconds.
fn until(what: &str, secs: u64, mut done: impl FnMut() -> bool) {
    let by = Instant::now() + Duration::from_secs(secs);
    while !done() {
        assert!(Instant::now() < by, "timed out waiting until {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// The worker wants a file only the sender has, before the sender has even
/// reached it, and it arrives whole over a direct connection.
#[test]
fn a_file_reaches_the_worker_directly() {
    let root = folder("direct");
    let (job, item) = (Uuid::now_v7(), Uuid::now_v7());
    let original = root.join("original.mov");
    let bytes = file(&original, 5 << 20);
    let worker_shelf = shelf(&root, "worker", Vec::new());
    let (worker_said, events) = heard();
    let worker = work(
        job,
        key(1),
        true,
        None,
        worker_shelf.clone(),
        limits(),
        events,
    )
    .unwrap();
    let wanted = Wanted::Original { item };
    worker.want(wanted.clone());

    let held = Held {
        item,
        bytes: bytes.len() as u64,
    };
    let (_, events) = heard();
    let giving = shelf(&root, "sender", vec![(item, original)]);
    let sender = send(
        job,
        key(1),
        worker.addresses(),
        None,
        vec![held],
        giving,
        limits(),
        events,
    )
    .unwrap();
    until("the original arrives", 60, || {
        worker_shelf.arrived.lock().contains(&wanted)
    });
    let path = worker_shelf.room(&wanted).unwrap();
    assert!(std::fs::read(path).unwrap() == bytes);
    assert_eq!(worker.holders(item), [(1, held.bytes)]);
    assert!(worker.linked() && sender.linked());
    assert_eq!(
        worker_said.lock()[..],
        [LinkEvent::Linked { relayed: false }]
    );

    drop((sender, worker));
    let _ = std::fs::remove_dir_all(root);
}

/// With no address of the worker's to reach, the two meet at a relay, and
/// the export comes back to the sender through it.
#[test]
fn the_export_comes_back_through_a_relay() {
    let root = folder("relay");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let relay = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let serving = thread::spawn(move || {
        lumit_relay::serve(&[listener], lumit_relay::Limits::default(), &stopping)
    });

    let job = Uuid::now_v7();
    let exported = root.join("export.mp4");
    let bytes = file(&exported, 1 << 20);
    let (worker_said, events) = heard();
    let making = shelf(&root, "worker", vec![(job, exported)]);
    let worker = work(job, key(2), false, Some(&relay), making, limits(), events).unwrap();
    assert!(worker.addresses().is_empty());

    let sender_shelf = shelf(&root, "sender", Vec::new());
    let (sender_said, events) = heard();
    let sender = send(
        job,
        key(2),
        Vec::new(),
        Some(&relay),
        Vec::new(),
        sender_shelf.clone(),
        limits(),
        events,
    )
    .unwrap();
    let wanted = Wanted::Export { job };
    sender.want(wanted.clone());
    until("the export arrives", 60, || {
        sender_shelf.arrived.lock().contains(&wanted)
    });
    let path = sender_shelf.room(&wanted).unwrap();
    assert!(std::fs::read(path).unwrap() == bytes);
    let relayed = LinkEvent::Linked { relayed: true };
    assert!(sender_said.lock().contains(&relayed));
    assert!(worker_said.lock().contains(&relayed));

    drop((sender, worker));
    stop.store(true, Ordering::Relaxed);
    serving.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

/// A link cut part-way through a file comes back by itself, and the file
/// carries on from where it was cut instead of starting again.
#[test]
fn a_cut_link_comes_back_and_the_file_carries_on() {
    let root = folder("resume");
    let (job, item) = (Uuid::now_v7(), Uuid::now_v7());
    let original = root.join("original.mov");
    let bytes = file(&original, 768 << 10);
    let worker_shelf = shelf(&root, "worker", Vec::new());
    let (worker_said, events) = heard();
    let worker = work(
        job,
        key(3),
        true,
        None,
        worker_shelf.clone(),
        limits(),
        events,
    )
    .unwrap();

    // Slow enough to be cut half way.
    let pace = limits();
    pace.set(128 << 10, 0);
    let held = Held {
        item,
        bytes: bytes.len() as u64,
    };
    let (_, events) = heard();
    let giving = shelf(&root, "sender", vec![(item, original)]);
    let addresses = worker.addresses();
    let sender = send(
        job,
        key(3),
        addresses,
        None,
        vec![held],
        giving,
        pace,
        events,
    )
    .unwrap();
    let wanted = Wanted::Original { item };
    worker.want(wanted.clone());

    let path = worker_shelf.room(&wanted).unwrap();
    let mut part = path.clone().into_os_string();
    part.push(".part");
    let part = PathBuf::from(part);
    let size = || std::fs::metadata(&part).map(|m| m.len()).ok();
    until("half the file is here", 30, || {
        size().is_some_and(|n| n >= 384 << 10)
    });
    worker.end.cut();

    // Begun again, the part would be cut back to nothing, and take a second
    // at this pace to grow past a third of the file.
    let mut least = u64::MAX;
    until("the rest arrives", 60, || {
        least = least.min(size().unwrap_or(u64::MAX));
        worker_shelf.arrived.lock().contains(&wanted)
    });
    assert!(least >= 256 << 10, "the file began again from {least}");
    assert!(std::fs::read(&path).unwrap() == bytes);
    let said = worker_said.lock().clone();
    let linked = LinkEvent::Linked { relayed: false };
    assert_eq!(said, [linked, LinkEvent::Unlinked, linked]);

    drop((sender, worker));
    let _ = std::fs::remove_dir_all(root);
}

/// A sender with the wrong key never links, and keeps trying no more often
/// than its wait allows.
#[test]
fn the_wrong_key_never_links_and_tries_are_spaced() {
    let root = folder("wrong");
    let job = Uuid::now_v7();
    let (worker_said, events) = heard();
    let worker = work(
        job,
        key(4),
        true,
        None,
        shelf(&root, "w", Vec::new()),
        limits(),
        events,
    )
    .unwrap();
    let (sender_said, events) = heard();
    let addresses = worker.addresses();
    let wrong = shelf(&root, "s", Vec::new());
    let sender = send(
        job,
        key(5),
        addresses,
        None,
        Vec::new(),
        wrong,
        limits(),
        events,
    )
    .unwrap();

    // Somewhere that hangs up at once, so a sender that did not wait would
    // be back hundreds of times.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let at = listener.local_addr().unwrap();
    let (_, events) = heard();
    let lost = shelf(&root, "l", Vec::new());
    let trying = send(
        job,
        key(4),
        vec![at],
        None,
        Vec::new(),
        lost,
        limits(),
        events,
    )
    .unwrap();
    let began = Instant::now();
    let mut tries = 0;
    while began.elapsed() < Duration::from_secs(4) {
        while listener.accept().is_ok() {
            tries += 1;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!((2..=6).contains(&tries), "{tries} tries in four seconds");
    assert!(!sender.linked() && !worker.linked() && !trying.linked());
    assert!(sender_said.lock().is_empty() && worker_said.lock().is_empty());

    drop((sender, worker, trying));
    let _ = std::fs::remove_dir_all(root);
}
