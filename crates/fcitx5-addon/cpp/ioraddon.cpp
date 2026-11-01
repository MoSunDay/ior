// ior fcitx5 addon: glue-only implementation. All IM logic is in Rust and
// reached through the C ABI declared in ioraddon.h.
#include "ioraddon.h"

#include <fcitx-utils/event.h>
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

IorEngine::IorEngine(fcitx::EventLoop* eventLoop) : raw_(ior_engine_new()) {
    if (raw_ == nullptr) {
        return;
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

    // Shift toggles the Chinese/English mode on RELEASE, and only when it
    // was not used as a modifier (no other key pressed while held). Press
    // events are recorded but never forwarded to ior_engine_process_key,
    // whose Shift handling toggles on press — enabling both would toggle
    // twice. Hosts must pick exactly one of the two mechanisms.
    if (sym == FcitxKey_Shift_L || sym == FcitxKey_Shift_R) {
        std::string committed;
        if (keyEvent.isRelease()) {
            if (shiftDown_ && !shiftUsed_) {
                const auto current = ior_engine_ascii_mode(raw_);
                ior_engine_set_ascii_mode(raw_, current != 0 ? 0u : 1u);
                committed = ior_engine_take_commit(raw_); // pending preedit
                keyEvent.filterAndAccept();
            }
            shiftDown_ = false;
        } else {
            shiftDown_ = true;
            shiftUsed_ = false;
        }
        updateUI(*ic);
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
    if (ior_engine_preedit(raw_)[0] == '\0' && !mayStartComposition(sym) &&
        !(chineseMode && isPunctKey(sym))) {
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

void IorEngine::reset(const fcitx::InputMethodEntry& /*entry*/,
                      fcitx::InputContextEvent& event) {
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
    return new IorEngine(m != nullptr ? m->eventLoop() : nullptr);
}

FCITX_ADDON_FACTORY(IorEngineFactory);
