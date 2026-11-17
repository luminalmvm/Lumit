//! Sending on a link. Runs on each link's sending thread.

use super::*;

impl Hub {
    /// Send what is queued for a link: frames as they come, and a file a
    /// piece at a time with any frame that turns up let past between pieces.
    pub(super) fn send_loop(&self, peer: u32, mut sender: Sender, outbox: &Queue<Out>) {
        let mut files: VecDeque<(u64, Wanted, PathBuf, Resume)> = VecDeque::new();
        let mut said = Instant::now();
        'link: loop {
            let next = if files.is_empty() {
                match outbox.recv_timeout(PING) {
                    Ok(next) => Some(next),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                outbox.try_recv().ok()
            };
            match next {
                Some(Out::Frame(bytes)) => {
                    if sender.send(&bytes).is_err() {
                        break;
                    }
                    said = Instant::now();
                    continue;
                }
                Some(Out::File {
                    request,
                    wanted,
                    path,
                    resume,
                }) => {
                    files.push_back((request, wanted, path, resume));
                    continue;
                }
                None if files.is_empty() => {
                    if said.elapsed() >= PING && sender.send(&[STILL_HERE]).is_err() {
                        break;
                    }
                    said = Instant::now();
                    continue;
                }
                None => {}
            }
            // One whole file, unless a frame wants out between its pieces.
            let Some((request, wanted, path, (have, start))) = files.pop_front() else {
                continue;
            };
            if self.unwanted(peer, request) {
                continue;
            }
            let opened = File::open(&path).and_then(|mut file| {
                let total = file.metadata()?.len();
                // Carried on from where the other end got to only if what
                // it has is how this file starts.
                let same = have > 0 && have <= total && start_of(&path, have) == Some(start);
                let from = if same { have } else { 0 };
                file.seek(SeekFrom::Start(from))?;
                Ok((file, total, from))
            });
            let Ok((mut file, total, from)) = opened else {
                if sender.send(&frame(NO, request, &[])).is_err() {
                    break;
                }
                continue;
            };
            let mut sizes = total.to_le_bytes().to_vec();
            sizes.extend_from_slice(&from.to_le_bytes());
            if sender.send(&frame(SIZE, request, &sizes)).is_err() {
                break;
            }
            let mut done = from;
            let mut told = Instant::now();
            let mut piece = vec![0u8; PIECE];
            let mut dropped = false;
            loop {
                if self.stopped() {
                    break 'link;
                }
                if self.unwanted(peer, request) {
                    dropped = true;
                    break;
                }
                while let Ok(between) = outbox.try_recv() {
                    match between {
                        Out::Frame(bytes) => {
                            if sender.send(&bytes).is_err() {
                                break 'link;
                            }
                        }
                        Out::File {
                            request,
                            wanted,
                            path,
                            resume,
                        } => files.push_back((request, wanted, path, resume)),
                    }
                }
                let Ok(n) = file.read(&mut piece) else {
                    break 'link;
                };
                if n == 0 {
                    break;
                }
                self.limits.up.take(n, &self.stop);
                if sender.send(&frame(DATA, request, &piece[..n])).is_err() {
                    break 'link;
                }
                done += n as u64;
                if told.elapsed() >= TELLING {
                    told = Instant::now();
                    self.footage.told(News::Moving {
                        wanted: wanted.clone(),
                        done,
                        total,
                        sending: true,
                    });
                }
            }
            // Given up on by the other end: nothing more is said of it.
            if dropped {
                continue;
            }
            if sender
                .send(&frame(END, request, &done.to_le_bytes()))
                .is_err()
            {
                break;
            }
            said = Instant::now();
            self.footage.told(News::Moving {
                wanted,
                done,
                total,
                sending: true,
            });
        }
        sender.close();
    }
}
