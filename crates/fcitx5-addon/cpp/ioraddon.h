// ior fcitx5 addon: thin C++ glue around the Rust engine (C ABI).
//
// The engine itself lives in Rust (crates/fcitx5-addon/src, crate
// `ior_engine`); this file only declares the mirrored C API and the fcitx5
// integration classes. Written for fcitx5 5.1.x; see cpp/README.md.
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
} // extern "C"

#include <fcitx/addonfactory.h>
#include <fcitx/inputmethodengine.h>
#include <fcitx/instance.h>

namespace fcitx {
class InputContext;
class InputMethodEntry;
class KeyEvent;
} // namespace fcitx

// fcitx5 input method engine backed by the Rust ior engine.
//
// M0 simplification: a single IorEngineC instance is shared by all input
// contexts (one user, one machine, one preedit at a time). Per-input-context
// state (e.g. a map from InputContext to engine) lands in M1.
class IorEngine final : public fcitx::InputMethodEngine {
public:
    explicit IorEngine(fcitx::Instance* instance);
    ~IorEngine() override;

    void keyEvent(const fcitx::InputMethodEntry& entry,
                  fcitx::KeyEvent& keyEvent) override;
    void reset(const fcitx::InputMethodEntry& entry,
               fcitx::InputContext& ic) override;

private:
    // Repaints preedit and candidate list for `ic` from the Rust engine.
    void updateUI(fcitx::InputContext& ic);

    fcitx::Instance* instance_;
    IorEngineC* raw_ = nullptr;
};

class IorEngineFactory final : public fcitx::AddonFactory {
public:
    fcitx::AddonInstance* create(fcitx::Instance* instance) override;
};

#endif // FCITX5_IOR_IORADDON_H_
