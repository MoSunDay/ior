// ior fcitx5 addon: thin C++ glue around the Rust engine (C ABI).
//
// The engine lives in Rust (crates/fcitx5-addon/src, crate `ior_engine`);
// this header mirrors the C API and declares the fcitx5 integration
// classes. Written against fcitx5 5.1.7 headers; see cpp/README.md.
#ifndef FCITX5_IOR_IORADDON_H_
#define FCITX5_IOR_IORADDON_H_

#include <cstdint>

// ---------------------------------------------------------------------------
// Mirror of the C ABI exported by crates/fcitx5-addon/src/ffi.rs.
// Keep this block in sync with that file (source of truth).
// The C-side opaque type is named IorEngineC because the fcitx5 engine
// class below already uses the name IorEngine.
//
// Contract:
//  - ior_engine_new returns nullptr only on allocation failure; all other
//    functions accept nullptr (process_key -> -1, preedit/take_commit -> "",
//    candidate_count -> 0, candidate -> nullptr).
//  - process_key returns 0 = forward to app, 1 = consumed (editing),
//    2 = committed (then call ior_engine_take_commit), -1 = null engine.
//  - Returned strings are UTF-8, NUL-terminated, never nullptr, and valid
//    only until the next ior_* call on the same engine. Never free them;
//    copy into std::string if the text must survive.
//  - The engine is single-threaded (fcitx5 main loop only).
// ---------------------------------------------------------------------------
extern "C" {
typedef struct IorEngineC IorEngineC;

IorEngineC* ior_engine_new(void);
void ior_engine_free(IorEngineC* engine);
void ior_engine_reset(IorEngineC* engine);
int32_t ior_engine_process_key(IorEngineC* engine, uint32_t keysym);
const char* ior_engine_preedit(const IorEngineC* engine);
uint32_t ior_engine_candidate_count(const IorEngineC* engine);
const char* ior_engine_candidate(const IorEngineC* engine, uint32_t index);
const char* ior_engine_take_commit(IorEngineC* engine);
uint32_t ior_engine_abi_version(void);
} // extern "C"

#include <fcitx/addonfactory.h>
#include <fcitx/candidatelist.h>
#include <fcitx/inputmethodengine.h>
#include <fcitx/text.h>

namespace fcitx {
class InputContext;
class KeyEvent;
} // namespace fcitx

// One shared engine instance across all input contexts is an accepted M0
// simplification; per-IC state lands in M1 (see docs/M0-FRONTEND-TEST.md).
class IorEngine final : public fcitx::InputMethodEngine {
public:
    IorEngine();
    ~IorEngine() override;

    void keyEvent(const fcitx::InputMethodEntry& entry,
                  fcitx::KeyEvent& keyEvent) override;
    void reset(const fcitx::InputMethodEntry& entry,
               fcitx::InputContextEvent& event) override;

    // Refreshes preedit + candidate list from the engine state. Public so
    // candidate words can refresh the panel after a mouse selection.
    void updateUI(fcitx::InputContext& ic);

private:
    IorEngineC* raw_;
};

// Selectable candidate word; select() routes a synthetic digit key through
// the engine so commit/learn logic stays in Rust.
class IorCandidateWord final : public fcitx::CandidateWord {
public:
    IorCandidateWord(fcitx::Text text, IorEngineC* engine, IorEngine* owner,
                     uint32_t index);
    void select(fcitx::InputContext* inputContext) const override;

private:
    IorEngineC* engine_;
    IorEngine* owner_;
    uint32_t index_;
};

class IorEngineFactory final : public fcitx::AddonFactory {
public:
    fcitx::AddonInstance* create(fcitx::AddonManager* manager) override;
};

#endif // FCITX5_IOR_IORADDON_H_
