//! Immutable session contexts and generation fences for late HTTP replies.

use crate::document::Draft;
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SessionContext {
    pub(crate) endpoint: String,
    pub(crate) principal: String,
    pub(crate) organization: String,
    pub(crate) workspace_selector: String,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DraftKey {
    pub(crate) context: SessionContext,
    pub(crate) workflow: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestStamp {
    generation: u64,
    sequence: u64,
}

#[derive(Default)]
pub(crate) struct Session {
    generation: u64,
    sequence: u64,
    pending: Option<RequestStamp>,
    /// The pending request writes the open draft (a save or publish), whose reply replaces it.
    writing: bool,
    pub(crate) context: Option<SessionContext>,
    pub(crate) selected: Option<DraftKey>,
    pub(crate) drafts: BTreeMap<DraftKey, Draft>,
}

impl Session {
    #[tracing::instrument(name = "client.session.switch", skip_all)]
    pub(crate) fn switch(&mut self, context: Option<SessionContext>) {
        self.generation += 1;
        self.pending = None;
        self.writing = false;
        self.context = context;
        self.selected = None;
    }
    pub(crate) fn begin(&mut self) -> Option<RequestStamp> {
        if self.pending.is_some() {
            return None;
        }
        self.sequence += 1;
        let stamp = RequestStamp {
            generation: self.generation,
            sequence: self.sequence,
        };
        self.pending = Some(stamp);
        self.writing = false;
        Some(stamp)
    }
    /// Begins a request that writes the open draft, such as a save or publish.
    pub(crate) fn begin_write(&mut self) -> Option<RequestStamp> {
        let stamp = self.begin()?;
        self.writing = true;
        Some(stamp)
    }
    #[tracing::instrument(name = "client.session.reply", skip_all)]
    pub(crate) fn accept(&mut self, stamp: RequestStamp) -> bool {
        if self.pending == Some(stamp) {
            self.pending = None;
            self.writing = false;
            true
        } else {
            false
        }
    }
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    /// A save or publish is in flight.
    pub(crate) fn writing(&self) -> bool {
        self.writing
    }
    pub(crate) fn draft(&self) -> Option<&Draft> {
        self.drafts.get(self.selected.as_ref()?)
    }
    pub(crate) fn draft_mut(&mut self) -> Option<&mut Draft> {
        self.drafts.get_mut(self.selected.as_ref()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_session_cannot_apply_or_clear_a_new_pending_request() {
        let mut session = Session::default();
        let old = session.begin().unwrap();
        session.switch(None);
        let current = session.begin().unwrap();
        assert!(!session.accept(old));
        assert!(session.busy());
        assert!(session.accept(current));
        assert!(!session.accept(current));
    }
    #[test]
    fn only_a_save_or_publish_in_flight_counts_as_writing() {
        let mut session = Session::default();
        let read = session.begin().unwrap();
        assert!(session.busy() && !session.writing());
        assert!(session.accept(read));

        let write = session.begin_write().unwrap();
        assert!(session.writing());
        assert!(session.begin_write().is_none());
        assert!(session.accept(write));
        assert!(!session.writing());

        session.begin_write().unwrap();
        session.switch(None);
        assert!(!session.writing());
    }
    #[test]
    fn reconnect_restores_only_the_originating_server_principal_workspace_draft() {
        let context = SessionContext {
            endpoint: "https://one.test/".into(),
            principal: "usr_one".into(),
            organization: "org_one".into(),
            workspace_selector: "ws_one".into(),
        };
        let key = DraftKey {
            context: context.clone(),
            workflow: "wf_test".into(),
        };
        let mut draft = Draft::new(crate::document::tests::snapshot(1, 7)).unwrap();
        draft.edit("echo", "message", "9").unwrap();
        draft.uncertain_save = true;
        let mut session = Session::default();
        session.drafts.insert(key.clone(), draft);
        session.switch(None);
        let mut other = context.clone();
        other.endpoint = "https://two.test/".into();
        assert!(!session.drafts.contains_key(&DraftKey {
            context: other,
            workflow: "wf_test".into()
        }));
        session.switch(Some(context));
        session.selected = Some(key);
        assert!(session.draft().unwrap().dirty());
        assert!(session.draft().unwrap().uncertain_save);
    }
}
