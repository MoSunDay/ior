// ior fcitx5 addon: glue only. All IME logic (segmentation, dictionary,
// candidates, learning) runs inside the Rust engine behind the C ABI.
#include "ioraddon.h"

#include <fcitx/inputcontext.h>
#include <fcitx/inputpanel.h>
#include <fcitx/key.h>
#include <fcitx/text.h>
#include <fcitx/candidatelist.h>

#include <memory>
#include <string>
#include <vector>

IorEngine::IorEngine(fcitx::Instance* instance) : instance_(instance) {
    raw_ = ior_engine_new();
}

IorEngine::~IorEngine() { ior_engine_free(raw_); }

void IorEngine::keyEvent(const fcitx::InputMethodEntry& /*entry*/,
                         fcitx::KeyEvent& keyEvent) {
    if (raw_ == nullptr || keyEvent.isRelease()) {
        return;
    }

    auto* ic = keyEvent.inputContext();
    const auto sym = static_cast<uint32_t>(keyEvent.key().sym());

    // Stay quiet unless we are composing or the key could start composing.
    const bool composing = (*ior_engine_preedit(raw_) != '\0');
    const bool isLetter = (sym >= 'A' && sym <= 'Z') ||
                          (sym >= 'a' && sym <= 'z');
    if (!composing && !isLetter) {
        return;
    }

    const auto result = ior_engine_process_key(raw_, sym);
    if (result == 0) {
        // Not ours: forward. If a leftover preedit existed (e.g. the key was
        // an uppercase letter), drop it and repaint before forwarding.
        if (*ior_engine_preedit(raw_) != '\0') {
            ior_engine_reset(raw_);
            updateUI(*ic);
        }
        return;
    }

    keyEvent.filter();
    updateUI(*ic);
    if (result == 2) {
        const char* text = ior_engine_take_commit(raw_);
        if (*text != '\0') {
            ic->commitString(text);
        }
    }
}

void IorEngine::reset(const fcitx::InputMethodEntry& /*entry*/,
                      fcitx::InputContext& ic) {
    if (raw_ != nullptr) {
        ior_engine_reset(raw_);
    }
    updateUI(ic);
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
        for (const auto& word : words) {
            list->append<fcitx::CommonCandidateWord>(fcitx::Text(word));
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

fcitx::AddonInstance* IorEngineFactory::create(fcitx::Instance* instance) {
    return new IorEngine(instance);
}

FCITX_ADDON_FACTORY_V2(ior, IorEngineFactory);
