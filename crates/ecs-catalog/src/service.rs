//! The catalog worker: a background thread with a single-threaded Tokio
//! runtime. The UI sends jobs and polls replies; nothing blocks the UI.
//! Every provider call goes through its [`ConsentGate`](crate::ConsentGate),
//! so a provider that is not approved answers `NotApproved` without a
//! request. [`Service::cancel`] aborts everything in flight (used when the
//! user turns catalogs or a provider off).

use crate::providers::{self, Env};
use crate::{
    CatalogError, CatalogProvider, CatalogQuery, ConsentGate, ProviderId, SearchPage, VendorPart,
};
use std::sync::Arc;
use std::sync::mpsc;

/// What a reply carries.
#[derive(Debug, Clone)]
pub enum Outcome {
    Page(Result<SearchPage, CatalogError>),
    Parts(Result<Vec<VendorPart>, CatalogError>),
}

/// One provider's answer to one job.
#[derive(Debug, Clone)]
pub struct Reply {
    pub job: u64,
    pub provider: ProviderId,
    pub outcome: Outcome,
}

enum Work {
    Search(CatalogQuery),
    Mpn(String, Option<String>),
    VendorPn(String),
}

enum Msg {
    Job(u64, Vec<ProviderId>, Arc<Work>),
    Cancel,
}

/// Called after each reply (the app asks egui for a repaint).
pub type Notify = Arc<dyn Fn() + Send + Sync>;

pub struct Service {
    tx: tokio::sync::mpsc::UnboundedSender<Msg>,
    rx: mpsc::Receiver<Reply>,
    next: u64,
    env: Env,
}

impl Service {
    pub fn start(env: Env, notify: Notify) -> Self {
        let (tx, mut jobs) = tokio::sync::mpsc::unbounded_channel::<Msg>();
        let (out, rx) = mpsc::channel::<Reply>();
        let gates = providers::all(&env);
        std::thread::Builder::new()
            .name("catalogs".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                rt.block_on(async move {
                    let mut running: Vec<tokio::task::JoinHandle<()>> = Vec::new();
                    while let Some(msg) = jobs.recv().await {
                        running.retain(|h| !h.is_finished());
                        match msg {
                            Msg::Cancel => {
                                for h in running.drain(..) {
                                    h.abort();
                                }
                            }
                            Msg::Job(id, which, work) => {
                                for p in which {
                                    let Some(gate) = gates.iter().find(|g| g.id() == p).cloned()
                                    else {
                                        continue;
                                    };
                                    let (work, out, notify) =
                                        (work.clone(), out.clone(), notify.clone());
                                    running.push(tokio::spawn(async move {
                                        let outcome = run(&gate, &work).await;
                                        let _ = out.send(Reply {
                                            job: id,
                                            provider: p,
                                            outcome,
                                        });
                                        notify();
                                    }));
                                }
                            }
                        }
                    }
                });
            })
            .expect("catalog thread");
        Self {
            tx,
            rx,
            next: 1,
            env,
        }
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    fn submit(&mut self, providers: &[ProviderId], work: Work) -> u64 {
        let id = self.next;
        self.next += 1;
        let _ = self
            .tx
            .send(Msg::Job(id, providers.to_vec(), Arc::new(work)));
        id
    }

    /// Search several providers; one reply per provider.
    pub fn search(&mut self, providers: &[ProviderId], q: CatalogQuery) -> u64 {
        self.submit(providers, Work::Search(q))
    }
    /// The same manufacturer part at several providers.
    pub fn by_mpn(
        &mut self,
        providers: &[ProviderId],
        mpn: &str,
        manufacturer: Option<&str>,
    ) -> u64 {
        self.submit(
            providers,
            Work::Mpn(mpn.into(), manufacturer.map(str::to_string)),
        )
    }
    /// One vendor part number at one provider (refresh).
    pub fn by_vendor_pn(&mut self, provider: ProviderId, pn: &str) -> u64 {
        self.submit(&[provider], Work::VendorPn(pn.into()))
    }
    /// Abort everything in flight; replies already queued are dropped.
    pub fn cancel(&mut self) {
        let _ = self.tx.send(Msg::Cancel);
        while self.rx.try_recv().is_ok() {}
    }
    /// Replies that arrived since the last poll.
    pub fn poll(&self) -> Vec<Reply> {
        self.rx.try_iter().collect()
    }
}

async fn run(gate: &ConsentGate, work: &Work) -> Outcome {
    match work {
        Work::Search(q) => Outcome::Page(gate.search(q).await.map(|page| SearchPage {
            parts: crate::filter(page.parts, q),
            total: page.total,
        })),
        Work::Mpn(mpn, mfr) => Outcome::Parts(gate.by_mpn(mpn, mfr.as_deref()).await),
        Work::VendorPn(pn) => Outcome::Parts(gate.by_vendor_pn(pn).await.map(|p| vec![p])),
    }
}
