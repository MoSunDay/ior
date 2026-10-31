// ior fcitx5 addon: glue-only implementation. All IM logic is in Rust and
// reached through the C ABI declared in ioraddon.h.
#include "ioraddon.h"

#include <fcitx-utils/key.h>
#include <fcitx/event.h>
#include <fcitx/inputcontext.h>
#include <fcitx/inputpanel.h>

#include <memory>
#include <string>
#include <vector>

namespace {
// Keysyms for keys that may start a composition. Everything else is
// forwarded untouched while no composition is active.
bool mayStartComposition(uint32_t sym) {
    return (sym >= 'a' && sym <= 'z') || (sym >= 'A' && sym <= 'Z');
}
} // namespace

IorEngine::IorEngine() : raw_(ior_engine_new()) {}
IorEngine::~IorEngine() { ior_engine_free(raw_); }

void IorEngine::keyEvent(const fcitx::InputMethodEntry& /*entry*/,
                         fcitx::KeyEvent& keyEvent) {
    if (raw_ == nullptr || keyEvent.isRelease()) {
        return;
    }
    auto* ic = keyEvent.inputContext();
    if (ic == nullptr) {
        return;
    }
    const auto sym = static_cast<uint32_t>(keyEvent.key().sym());
    if (ior_engine_preedit(raw_)[0] == '\0' && !mayStartComposition(sym)) {
        return; // idle: let every non-letter key reach the app
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
    const char* preedit = ior_engine_preedit(raw_);
    if (*preedit == '\0') {
        panel.reset();
    } else {
        panel.setClientPreedit(
            fcitx::Text(preedit, fcitx::TextFormatFlag::Underline));

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
        list->setPageSize(5);
        if (!words.empty()) {
            list->setGlobalCursorIndex(0);
        }
        panel.setCandidateList(std::move(list));
    }
    ic.updateUserInterface(fcitx::UserInterfaceComponent::InputPanel);
}

IorCandidateWord::IorCandidateWord(fcitx::Text text, IorEngineC* engine,
                                   IorEngine* owner, uint32_t index)
    : fcitx::CandidateWord(std::move(text)), engine_(engine), owner_(owner),
      index_(index) {}

void IorCandidateWord::select(fcitx::InputContext* ic) const {
    if (engine_ == nullptr || ic == nullptr) {
        return;
    }
    // Reuse the engine's digit-selection path (keysym '1'..'9', '0').
    const uint32_t sym =
        index_ < 9 ? static_cast<uint32_t>('1') + index_ : static_cast<uint32_t>('0');
    if (ior_engine_process_key(engine_, sym) == 2) {
        const char* text = ior_engine_take_commit(engine_);
        if (*text != '\0') {
            ic->commitString(text);
        }
    }
    owner_->updateUI(*ic);
}

fcitx::AddonInstance* IorEngineFactory::create(fcitx::AddonManager* /*m*/) {
    return new IorEngine();
}

FCITX_ADDON_FACTORY(IorEngineFactory);
