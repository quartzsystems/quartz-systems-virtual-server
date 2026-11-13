// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The thread that serves a device's queues. It sleeps on the queues'
//! notifications and on a stop signal; a notification runs the
//! device's handler for that queue. A reset stops and joins it.

use std::io;
use std::os::fd::AsRawFd;
use std::thread::{self, JoinHandle};

use vmm_sys_util::epoll::{ControlOperation, Epoll, EpollEvent, EventSet};
use vmm_sys_util::eventfd::EventFd;

/// The epoll key of the stop signal: no queue has this number.
const STOP: u64 = u64::MAX;

/// A running worker.
pub struct Worker {
    stop: EventFd,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Serve on a thread called `name`: `handler` is called with a
    /// queue's number whenever that queue's eventfd is written.
    pub fn spawn<F>(name: &str, notifies: Vec<(usize, EventFd)>, mut handler: F) -> io::Result<Self>
    where
        F: FnMut(usize) + Send + 'static,
    {
        let stop = EventFd::new(0)?;
        let epoll = Epoll::new()?;
        epoll.ctl(
            ControlOperation::Add,
            stop.as_raw_fd(),
            EpollEvent::new(EventSet::IN, STOP),
        )?;
        for (index, fd) in &notifies {
            epoll.ctl(
                ControlOperation::Add,
                fd.as_raw_fd(),
                EpollEvent::new(EventSet::IN, *index as u64),
            )?;
        }
        let thread = thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                let mut events = vec![EpollEvent::default(); notifies.len() + 1];
                loop {
                    let ready = match epoll.wait(-1, &mut events) {
                        Ok(n) => n,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(_) => return,
                    };
                    for event in &events[..ready] {
                        if event.data() == STOP {
                            return;
                        }
                        let found = notifies.iter().find(|(i, _)| *i as u64 == event.data());
                        if let Some((index, fd)) = found {
                            let _ = fd.read();
                            handler(*index);
                        }
                    }
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// Stop the thread and wait for it.
    pub fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.stop.write(1);
            let _ = thread.join();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn a_notification_runs_the_handler_and_stop_ends_the_thread() {
        let a = EventFd::new(0).unwrap();
        let b = EventFd::new(0).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut worker = Worker::spawn(
            "test",
            vec![(0, a.try_clone().unwrap()), (2, b.try_clone().unwrap())],
            move |index| tx.send(index).unwrap(),
        )
        .unwrap();
        b.write(1).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        a.write(1).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
        worker.stop();
        a.write(1).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        worker.stop();
    }
}
