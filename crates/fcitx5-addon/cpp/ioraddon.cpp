// ior fcitx5 addon: glue-only implementation. All IM logic is in Rust and
// reached through the C ABI declared in ioraddon.h.
#include "ioraddon.h"

#include <fcitx-utils/event.h>
#include <fcitx/action.h>
#include <fcitx/instance.h>
#include <fcitx/statusarea.h>
#include <fcitx-utils/key.h>
#include <fcitx/addonmanager.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputpanel.h>

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <ctime>
#include <cstdlib>
#include <filesystem>
#include <limits>
#include <memory>
#include <string>
#include <system_error>
#include <vector>

namespace {
// Keysyms for keys that may start a composition. Everything else is
// forwarded untouched while no composition is active.
bool mayStartComposition(uint32_t sym) {
    return (sym >= 'a' && sym <= 'z') || (sym >= 'A' && sym <= 'Z');
}

// Printable ASCII punctuation (not letters, not digits): in Chinese mode
// the engine maps these to their Chinese counterparts even while idle.
bool isPunctKey(uint32_t sym) {
    if (sym < 0x21 || sym > 0x7E) {
        return false;
    }
    const auto ch = static_cast<char>(sym);
    return !(ch >= '0' && ch <= '9') && !(ch >= 'a' && ch <= 'z') &&
           !(ch >= 'A' && ch <= 'Z');
}

// Learned frequencies halve for every 30 days since the last save (whole
// halvings only; entries that reach zero are dropped inside the engine).
constexpr uint32_t kUserdbHalfLifeSecs = 30u * 24u * 3600u;

// Clamps to uint32_t for the FFI (saturating at both ends).
uint32_t clampU32(int64_t secs) {
    if (secs <= 0) {
        return 0;
    }
    if (secs > static_cast<int64_t>(std::numeric_limits<uint32_t>::max())) {
        return std::numeric_limits<uint32_t>::max();
    }
    return static_cast<uint32_t>(secs);
}

// User dictionary file: $XDG_DATA_HOME (or $HOME/.local/share) +
// /fcitx5/ior/userdb.tsv. The directory is created on first use. Returns an
// empty path when the location cannot be determined or created — callers
// then skip persistence silently, because typing must never break on a
// unwritable disk.
std::filesystem::path userdbPath() {
    const char* xdg = std::getenv("XDG_DATA_HOME");
    std::string base;
    if (xdg != nullptr && *xdg != '\0') {
        base = xdg;
    } else {
        const char* home = std::getenv("HOME");
        if (home == nullptr || *home == '\0') {
            return {};
        }
        base = std::string(home) + "/.local/share";
    }
    const auto dir = std::filesystem::path(base) / "fcitx5" / "ior";
    std::error_code ec;
    std::filesystem::create_directories(dir, ec);
    if (ec) {
        return {};
    }
    return dir / "userdb.tsv";
}
} // namespace

// Loads the persisted user dictionary (a missing file is a normal cold
// start) and ages it by the time since it was last written: every 30 days
// since the last save halves the learned frequencies once.
// Voice poll cadence: 50ms is far below human perception of commit
// latency while staying negligible on the event loop.
constexpr uint64_t kVoicePollUsec = 50'000;
// How long the "Chinese"/"English" toggle hint stays on the panel.
constexpr uint64_t kModeHintUsec = 1'000'000;

IorEngine::IorEngine(fcitx::Instance* instance, fcitx::EventLoop* eventLoop)
    : raw_(ior_engine_new()), eventLoop_(eventLoop), instance_(instance) {
    if (raw_ == nullptr) {
        return;
    }
    if (instance_ != nullptr) {
        // Status-area toggle (tray icon menu; classicui renders status
        // area actions in the tray, not on the candidate window);
        // activate() re-adds it to every input context's status area.
        widthAction_ = std::make_unique<fcitx::SimpleAction>();
        widthAction_->setShortText("Full-width");
        widthAction_->setLongText("Toggle full-width input");
        widthAction_->setCheckable(true);
        widthAction_->setChecked(ior_engine_full_width(raw_) != 0);
        widthAction_->connect<fcitx::SimpleAction::Activated>(
            [this](fcitx::InputContext* ic) { toggleWidth(ic); });
        widthAction_->registerAction("ior-fullwidth",
                                     &instance_->userInterfaceManager());
        // One-shot "delete learned word" mode: clicking a candidate then
        // removes it from the user dictionary instead of committing.
        deleteAction_ = std::make_unique<fcitx::SimpleAction>();
        deleteAction_->setShortText("Del word");
        deleteAction_->setLongText(
            "Delete a learned word (click it in the candidate list)");
        deleteAction_->connect<fcitx::SimpleAction::Activated>(
            [this](fcitx::InputContext* ic) { toggleDeleteMode(ic); });
        deleteAction_->registerAction("ior-delword",
                                      &instance_->userInterfaceManager());
    }
    if (eventLoop != nullptr && ior_engine_voice_hotkey_keysym(raw_) != 0) {
        voicePollEvent_ = eventLoop->addTimeEvent(
            CLOCK_MONOTONIC, kVoicePollUsec, kVoicePollUsec / 5,
            [this](fcitx::EventSourceTime* source, uint64_t /*usec*/) {
                pollVoice();
                source->setNextInterval(kVoicePollUsec); // periodic
                return true;
            });
    }
    const auto path = userdbPath();
    if (path.empty() || ior_engine_userdb_load(raw_, path.c_str()) != 0) {
        return; // no usable history: start with an empty user dictionary
    }
    std::error_code ec;
    const auto mtime = std::filesystem::last_write_time(path, ec);
    if (ec) {
        return; // mtime unavailable: skip the decay, keep the frequencies
    }
    const auto elapsed = std::chrono::duration_cast<std::chrono::seconds>(
        std::filesystem::file_time_type::clock::now() - mtime);
    ior_engine_userdb_decay(raw_, clampU32(elapsed.count()),
                            kUserdbHalfLifeSecs);
}
IorEngine::~IorEngine() {
    // Exit fallback: persist whatever was learned but not saved yet.
    saveUserdb();
    ior_engine_free(raw_);
}

// Persists the learned dictionary right after it changed (every commit
// path ends here). Writing the snapshot is also compaction; failure is
// silent on purpose — persistence must never block input.
void IorEngine::saveUserdb() {
    if (raw_ == nullptr) {
        return;
    }
    const auto path = userdbPath();
    if (path.empty()) {
        return;
    }
    ior_engine_userdb_save(raw_, path.c_str());
}

void IorEngine::keyEvent(const fcitx::InputMethodEntry& /*entry*/,
                         fcitx::KeyEvent& keyEvent) {
    if (raw_ == nullptr) {
        return;
    }
    auto* ic = keyEvent.inputContext();
    if (ic == nullptr) {
        return;
    }
    lastIc_ = ic;
    const auto sym = static_cast<uint32_t>(keyEvent.key().sym());

    // ctrl+` (default) drives voice input before anything else, so it
    // works mid-composition and never leaks the grave key to the app.
    if (handleVoiceHotkey(ic, keyEvent)) {
        return;
    }

    // Delete-learned-word mode ends with the first key press: Esc just
    // closes it (the composition survives), every other key falls
    // through to normal processing. Key releases are none of its
    // business (a pending Shift release must still run its own path).
    if (deleteMode_ && !keyEvent.isRelease()) {
        exitDeleteMode();
        if (sym == FcitxKey_Escape) {
            keyEvent.filterAndAccept();
            updateUI(*ic);
            return;
        }
    }

    // Shift toggles the Chinese/English mode on RELEASE, and only when it
    // was not used as a modifier (no other key pressed while held). Both
    // the press and the release are consumed here and never forwarded to
    // ior_engine_process_key, whose Shift handling toggles on press —
    // enabling both would toggle twice.
    //
    // NOTE: fcitx5's global AltTriggerKeys (Shift_L by default) is checked
    // BEFORE engine dispatch, so it would switch the whole input method
    // away from ior before this handler ever sees the key; filterAndAccept
    // cannot win that race. Users must clear [Hotkey/AltTriggerKeys] in
    // ~/.config/fcitx5/config (see README) — with that cleared, Shift
    // reaches this handler and stays inside ior.
    if (sym == FcitxKey_Shift_L || sym == FcitxKey_Shift_R) {
        std::string committed;
        bool toggled = false;
        if (keyEvent.isRelease()) {
            if (shiftDown_ && !shiftUsed_) {
                const auto current = ior_engine_ascii_mode(raw_);
                ior_engine_set_ascii_mode(raw_, current != 0 ? 0u : 1u);
                committed = ior_engine_take_commit(raw_); // pending preedit
                toggled = true;
                keyEvent.filterAndAccept();
            } else {
                keyEvent.filter(); // orphan release: its press was consumed
            }
            shiftDown_ = false;
        } else {
            shiftDown_ = true;
            shiftUsed_ = false;
            keyEvent.filterAndAccept(); // keep AltTrigger away from Shift
        }
        updateUI(*ic);
        if (toggled) {
            showHint(*ic, ior_engine_ascii_mode(raw_) != 0 ? "English"
                                                           : "Chinese");
        }
        if (!committed.empty()) {
            ic->commitString(committed);
            saveUserdb(); // mode switch may have committed (and learned)
        }
        return;
    }
    if (keyEvent.isRelease()) {
        return;
    }
    // Any other key while Shift is held means Shift worked as a modifier.
    if (shiftDown_) {
        shiftUsed_ = true;
    }

    // Configurable paging keys ([engine] prev/next_page_keys; PageUp/Down
    // and '-'/'=' by default) act only while composing — idle they keep
    // their application/punctuation meaning, matching rime's
    // `when: paging` / `when: has_menu` bindings. Paging stays cyclic.
    if (ior_engine_preedit(raw_)[0] != '\0') {
        const auto pageDir = ior_engine_page_key_direction(raw_, sym);
        if (pageDir != 0) {
            if (pageDir < 0) {
                ior_engine_page_prev(raw_);
            } else {
                ior_engine_page_next(raw_);
            }
            keyEvent.filterAndAccept();
            updateUI(*ic);
            return;
        }
    }

    // Idle gate: letters may start a composition; in Chinese mode
    // punctuation reaches the engine too (it commits the mapped Chinese
    // punctuation). Everything else passes straight through.
    const bool chineseMode = ior_engine_ascii_mode(raw_) == 0;
    const bool fullWidth = ior_engine_full_width(raw_) != 0;
    const bool digit = sym >= '0' && sym <= '9';
    if (ior_engine_preedit(raw_)[0] == '\0' && !mayStartComposition(sym) &&
        !(chineseMode && isPunctKey(sym)) &&
        // Full-width: idle digits/space reach the engine, which commits
        // their full-width forms (punctuation follows chinese_punct and
        // is covered by the clause above).
        !(fullWidth && (digit || sym == ' '))) {
        return;
    }

    const auto result = ior_engine_process_key(raw_, sym);
    if (result == 0) { // not handled: forward, drop any stale composition
        if (ior_engine_preedit(raw_)[0] != '\0') {
            ior_engine_reset(raw_);
            updateUI(*ic);
        }
        return;
    }
    keyEvent.filterAndAccept();
    updateUI(*ic);
    if (result == 2) {
        const char* text = ior_engine_take_commit(raw_);
        if (*text != '\0') {
            ic->commitString(text);
            saveUserdb(); // learned on commit
        }
    }
}

void IorEngine::activate(const fcitx::InputMethodEntry& /*entry*/,
                         fcitx::InputContextEvent& event) {
    if (widthAction_ != nullptr && event.inputContext() != nullptr) {
        event.inputContext()->statusArea().addAction(
            fcitx::StatusGroup::InputMethod, widthAction_.get());
    }
    if (deleteAction_ != nullptr && event.inputContext() != nullptr) {
        event.inputContext()->statusArea().addAction(
            fcitx::StatusGroup::InputMethod, deleteAction_.get());
    }
}

void IorEngine::toggleWidth(fcitx::InputContext* ic) {
    if (raw_ == nullptr) {
        return;
    }
    const auto now = ior_engine_set_full_width(
        raw_, ior_engine_full_width(raw_) != 0 ? 0u : 1u);
    if (widthAction_ != nullptr) {
        widthAction_->setChecked(now != 0);
        if (ic != nullptr) {
            widthAction_->update(ic);
        }
    }
    if (ic != nullptr) {
        updateUI(*ic);
        showHint(*ic, now != 0 ? "Full-width" : "Half-width");
    }
}

// Status-area callback: enters (or cancels) the one-shot
// delete-learned-word mode. The panel announces it via an aux line while
// composing; the next candidate click deletes instead of committing.
void IorEngine::toggleDeleteMode(fcitx::InputContext* ic) {
    if (raw_ == nullptr) {
        return;
    }
    deleteMode_ = !deleteMode_;
    if (deleteAction_ != nullptr && ic != nullptr) {
        deleteAction_->update(ic);
    }
    if (ic != nullptr) {
        updateUI(*ic);
        showHint(*ic, deleteMode_
                         ? "Click a candidate to delete it (Esc exits)"
                         : "Delete mode off");
    }
}

void IorEngine::reset(const fcitx::InputMethodEntry& /*entry*/,
                      fcitx::InputContextEvent& event) {
    exitDeleteMode();
    if (raw_ != nullptr) {
        ior_engine_reset(raw_);
    }
    updateUI(*event.inputContext());
}

void IorEngine::updateUI(fcitx::InputContext& ic) {
    if (raw_ == nullptr) {
        return;
    }
    auto& panel = ic.inputPanel();
    const uint32_t voiceState = ior_engine_voice_state(raw_);
    // Copy the FFI string before other ior_* calls (lifetime rule).
    const std::string preedit = ior_engine_preedit(raw_);
    if (preedit.empty() && voiceState == 0) {
        panel.reset();
    } else {
        std::string line = preedit;
        if (voiceState != 0) {
            // While listening/transcribing the panel shows a status hint
            // instead of the (empty) composition preedit.
            const std::string hotkey = ior_engine_voice_hotkey_name(raw_);
            line = voiceState == 1 ? "[voice] listening... press " + hotkey +
                                         " again to cancel"
                                   : "[voice] transcribing...";
        }
        panel.setPreedit(
            fcitx::Text(line, fcitx::TextFormatFlag::Underline));
        if (voiceState != 0) {
            ic.updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
            return; // no candidates while the mic is open
        }
        // Delete mode rides along as an aux line so the click target is
        // announced right above the candidates it will act on. Any other
        // aux (a transient "Chinese"/"English" mode hint) is cleared
        // here: a hint must not outlive the next refresh, or it reads
        // like a prefix stuck above the composition.
        panel.setAuxUp(deleteMode_
                           ? fcitx::Text("Click a candidate to delete it "
                                         "from the user dictionary "
                                         "(Esc exits)")
                           : fcitx::Text());
        // The panel preedit above keeps the raw pinyin out of the client's
        // text field; the candidate window still shows the composition.

        // FFI strings only live until the next ior_* call, so copy them
        // into std::string first and build the candidate list afterwards.
        std::vector<std::string> words;
        const auto count = ior_engine_candidate_count(raw_);
        words.reserve(count);
        for (uint32_t i = 0; i < count; ++i) {
            words.emplace_back(ior_engine_candidate(raw_, i));
        }

        auto list = std::make_unique<fcitx::CommonCandidateList>();
        for (uint32_t i = 0; i < words.size(); ++i) {
            list->append(std::make_unique<IorCandidateWord>(
                fcitx::Text(words[i]), raw_, this, i));
        }
        static const std::vector<fcitx::Key> selectionKeys = {
            fcitx::Key(FcitxKey_1), fcitx::Key(FcitxKey_2),
            fcitx::Key(FcitxKey_3), fcitx::Key(FcitxKey_4),
            fcitx::Key(FcitxKey_5), fcitx::Key(FcitxKey_6),
            fcitx::Key(FcitxKey_7), fcitx::Key(FcitxKey_8),
            fcitx::Key(FcitxKey_9), fcitx::Key(FcitxKey_0),
        };
        list->setSelectionKey(selectionKeys);
        // The engine already paginates (candidate_count is the current
        // page); expose it as one single fcitx page so the panel's own
        // paging can never desync from ior_engine_page_next/prev.
        list->setPageSize(std::max<size_t>(1, words.size()));
        if (!words.empty()) {
            list->setGlobalCursorIndex(0);
        }
        panel.setCandidateList(std::move(list));
    }
    ic.updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
}

// Brief panel hint after a mode toggle ("Chinese"/"English" on Shift,
// "Full-width"/"Half-width" on the status-area action), akin to fcitx5's
// own input-method-switch popup. A one-shot timer (replacing any pending
// one) clears it; while composing the engine state wins and the hint
// simply fades with the next updateUI.
void IorEngine::showHint(fcitx::InputContext& ic, const std::string& text) {
    if (raw_ == nullptr) {
        return;
    }
    ic.inputPanel().setAuxUp(fcitx::Text(text));
    ic.updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
    if (eventLoop_ == nullptr) {
        return; // no loop (tests): hint stays until the next updateUI
    }
    const auto watcher = ic.watch(); // survive an IC destroyed meanwhile
    modeHintEvent_ = eventLoop_->addTimeEvent(
        CLOCK_MONOTONIC, fcitx::now(CLOCK_MONOTONIC) + kModeHintUsec,
        kModeHintUsec / 10,
        [this, watcher](fcitx::EventSourceTime* /*source*/, uint64_t) {
            auto* live = watcher.get();
            if (live != nullptr && raw_ != nullptr) {
                if (ior_engine_voice_state(raw_) == 0 &&
                    ior_engine_preedit(raw_)[0] == '\0') {
                    live->inputPanel().reset();
                } else {
                    // Composing or listening: keep the panel content,
                    // drop only the hint line itself.
                    live->inputPanel().setAuxUp(fcitx::Text());
                }
                live->updateUserInterface(
                    fcitx::UserInterfaceComponent::InputPanel);
            }
            return false; // one-shot
        });
}

// True when the key event is the configured voice hotkey (sym + exact
// modifier bits over Shift/Ctrl/Alt/Super; NumLock etc. are ignored).
bool IorEngine::handleVoiceHotkey(fcitx::InputContext* ic,
                                  fcitx::KeyEvent& keyEvent) {
    const auto sym = static_cast<uint32_t>(keyEvent.key().sym());
    const uint32_t hotkeySym = ior_engine_voice_hotkey_keysym(raw_);
    if (hotkeySym == 0 || sym != hotkeySym) {
        return false;
    }
    constexpr uint32_t kModMask = (1u << 0) /*Shift*/ | (1u << 2) /*Ctrl*/ |
                                  (1u << 3) /*Alt*/ | (1u << 6) /*Super*/;
    const uint32_t states = keyEvent.key().states().toInteger();
    if ((states & kModMask) != ior_engine_voice_hotkey_mods(raw_)) {
        return false; // e.g. plain ` or ctrl+shift+`: not the hotkey
    }
    const bool hold = ior_engine_voice_hotkey_mode(raw_) != 0;
    if (!keyEvent.isRelease()) {
        // toggle: idle -> record, active -> cancel.
        // hold: start when idle, otherwise ignore the repeat press.
        if (!hold || ior_engine_voice_state(raw_) == 0) {
            ior_engine_voice_toggle(raw_);
        }
    } else if (hold && ior_engine_voice_state(raw_) == 1) {
        ior_engine_voice_stop(raw_); // release stops + transcribes
    } else {
        return false; // releases in toggle mode are not ours
    }
    keyEvent.filterAndAccept();
    pollVoice(); // pick up an immediate cancel transition
    updateUI(*ic);
    return true;
}

// Drains the worker: commits a finished transcript to lastIc_ (bypassing
// the pinyin session — ASR output is final Hanzi) and refreshes the UI.
void IorEngine::pollVoice() {
    if (raw_ == nullptr) {
        return;
    }
    if (ior_engine_voice_poll(raw_) != 1) {
        return;
    }
    // Copy before any other ior_* call (FFI lifetime rule).
    const std::string text = ior_engine_voice_take_text(raw_);
    if (text.empty()) {
        return;
    }
    if (ior_engine_preedit(raw_)[0] != '\0') {
        ior_engine_reset(raw_); // drop a stale composition first
    }
    if (lastIc_ != nullptr) {
        lastIc_->commitString(text); // not learned: ASR text is not pinyin
        updateUI(*lastIc_);
    }
}

IorCandidateWord::IorCandidateWord(fcitx::Text text, IorEngineC* engine,
                                   IorEngine* owner, uint32_t index)
    : fcitx::CandidateWord(std::move(text)), engine_(engine), owner_(owner),
      index_(index) {}

void IorCandidateWord::select(fcitx::InputContext* ic) const {
    if (engine_ == nullptr || ic == nullptr) {
        return;
    }
    // Delete-learned-word mode: this click removes the learned entry
    // behind the candidate (page-local index, same address space as the
    // commit path below) instead of committing it. The mode is one-shot
    // and the composition survives, so typing continues right after.
    if (owner_ != nullptr && owner_->inDeleteMode()) {
        owner_->exitDeleteMode();
        const std::string word = text().toString();
        const auto result = ior_engine_candidate_delete(engine_, index_);
        if (result == 0) {
            owner_->saveUserdb(); // deletion changes the persisted userdb
            owner_->updateUI(*ic);
            owner_->showHint(*ic, "Deleted learned word " + word);
        } else {
            owner_->updateUI(*ic);
            owner_->showHint(*ic,
                             word + " is not a learned word (result " +
                                 std::to_string(result) + ")");
        }
        return;
    }
    // Commit by page-local index (ABI v2): what the panel displays is what
    // gets committed, on every page — no synthetic digit keys that would
    // select the wrong row after paging.
    const char* text = ior_engine_select(engine_, index_);
    if (*text != '\0') {
        ic->commitString(text);
        owner_->saveUserdb(); // learned on commit
    }
    owner_->updateUI(*ic);
}

fcitx::AddonInstance* IorEngineFactory::create(fcitx::AddonManager* m) {
    return new IorEngine(m != nullptr ? m->instance() : nullptr,
                         m != nullptr ? m->eventLoop() : nullptr);
}

FCITX_ADDON_FACTORY(IorEngineFactory);
