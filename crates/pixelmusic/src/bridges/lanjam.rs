// LanJam — mirror of LanJamService: the same-Wi-Fi Jam (no account). state: LanJamSnapshot keys (status, isHost,
// roomCode, hostName, members, pendingApproval, canControl, canAddTracks, requireApproval, address, inviteUrl,
// error, busy, nearby, discovering).
use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use sf_services::lanjam::LanJamSnapshot;
use sf_services::lanjam::protocol::JamControl;

use super::{forward_watch, to_qmap};
use crate::context::ctx;
use crate::qt::QVariantMap;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qmap.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qproperty(QMap_QString_QVariant, state, READ, NOTIFY = state_changed)]
        #[qproperty(bool, active, READ, NOTIFY = state_changed)]
        type LanJam = super::LanJamRust;

        #[qsignal]
        #[cxx_name = "stateChanged"]
        fn state_changed(self: Pin<&mut LanJam>);

        /// Host a Jam on this computer ("" — the computer's name).
        #[qinvokable]
        fn host(self: &LanJam, name: &QString);
        /// Join by room code or invite link.
        #[qinvokable]
        fn join(self: &LanJam, input: &QString, name: &QString);
        /// Leave (a host ends the Jam).
        #[qinvokable]
        fn leave(self: &LanJam);
        #[qinvokable]
        fn approve(self: &LanJam, user_id: &QString);
        #[qinvokable]
        fn remove(self: &LanJam, user_id: &QString);
        #[qinvokable]
        #[cxx_name = "setRoomSettings"]
        fn set_room_settings(self: &LanJam, can_add_tracks: bool, can_control: bool, require_approval: bool);
        /// List rooms on this network (while the card is shown).
        #[qinvokable]
        #[cxx_name = "startDiscovery"]
        fn start_discovery(self: &LanJam);
        #[qinvokable]
        #[cxx_name = "stopDiscovery"]
        fn stop_discovery(self: &LanJam);
        /// Guest: ask the host to play/pause, skip.
        #[qinvokable]
        #[cxx_name = "togglePlay"]
        fn toggle_play(self: &LanJam);
        #[qinvokable]
        fn next(self: &LanJam);
        #[qinvokable]
        fn previous(self: &LanJam);
        /// Copy the invite link; false — none.
        #[qinvokable]
        #[cxx_name = "copyInvite"]
        fn copy_invite(self: &LanJam) -> bool;
    }

    impl cxx_qt::Threading for LanJam {}
    impl cxx_qt::Initialize for LanJam {}
}

#[derive(Default)]
pub struct LanJamRust {
    snapshot: LanJamSnapshot,
    state: QVariantMap,
    active: bool,
}

impl qobject::LanJam {
    fn host(&self, name: &QString) {
        ctx().services.lanjam.host(&name.to_string());
    }

    fn join(&self, input: &QString, name: &QString) {
        ctx().services.lanjam.join(&input.to_string(), &name.to_string());
    }

    fn leave(&self) {
        ctx().services.lanjam.leave();
    }

    fn approve(&self, user_id: &QString) {
        ctx().services.lanjam.approve(&user_id.to_string());
    }

    fn remove(&self, user_id: &QString) {
        ctx().services.lanjam.remove(&user_id.to_string());
    }

    fn set_room_settings(&self, can_add_tracks: bool, can_control: bool, require_approval: bool) {
        ctx().services.lanjam.set_room_settings(can_add_tracks, can_control, require_approval);
    }

    fn start_discovery(&self) {
        ctx().services.lanjam.start_discovery();
    }

    fn stop_discovery(&self) {
        ctx().services.lanjam.stop_discovery();
    }

    fn toggle_play(&self) {
        ctx().services.lanjam.toggle_play();
    }

    fn next(&self) {
        ctx().services.lanjam.control(JamControl::SkipNext);
    }

    fn previous(&self) {
        ctx().services.lanjam.control(JamControl::SkipPrevious);
    }

    fn copy_invite(&self) -> bool {
        let link = self.rust().snapshot.invite_url.clone();
        if link.is_empty() {
            return false;
        }
        crate::qt::copy_to_clipboard(&link);
        true
    }

    fn apply(mut self: Pin<&mut Self>, next: LanJamSnapshot) {
        if self.rust().snapshot == next {
            return;
        }
        let mut this = self.as_mut().rust_mut();
        this.state = to_qmap(&next);
        this.active = next.status != "idle";
        this.snapshot = next;
        self.as_mut().state_changed();
    }
}

impl cxx_qt::Initialize for qobject::LanJam {
    fn initialize(mut self: Pin<&mut Self>) {
        let mut watch = ctx().services.lanjam.watch();
        let snapshot = watch.borrow_and_update().clone();
        let mut this = self.as_mut().rust_mut();
        this.state = to_qmap(&snapshot);
        this.active = snapshot.status != "idle";
        this.snapshot = snapshot;
        forward_watch(self.qt_thread(), watch, Self::apply);
    }
}
