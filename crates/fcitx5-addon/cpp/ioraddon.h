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
// ABI version: 2 (check ior_engine_abi_version() after loading).
//
// Contract:
//  - ior_engine_new returns nullptr only on allocation failure; all other
//    functions accept nullptr (process_key -> -1, preedit/take_commit/
//    select -> "", candidate_count/page_info/ascii_mode -> 0,
//    page_next/page_prev -> 0, candidate -> nullptr).
//  - process_key returns 0 = forward to app, 1 = consumed (editing),
//    2 = committed (then call ior_engine_take_commit), -1 = null engine.
//  - Candidate symbols expose the CURRENT PAGE only; page_next/page_prev
//    page cyclically, page_info packs (current+1)<<16 | count (0 = none),
//    select commits by page-local index (candidate clicks) and returns the
//    committed text ("" for an invalid index).
//  - ascii_mode/set_ascii_mode report/force Chinese(0)/English(1); a mode
//    change runs the core Shift transition (pending preedit handled per
//    shift_action). NOTE: process_key ALSO toggles the mode on a Shift
//    press keysym; this shell uses the release-side path below and never
//    forwards Shift keys to process_key (pick ONE mechanism).
//  - Returned strings are UTF-8, NUL-terminated, never nullptr, and valid
//    only until the next ior_* call on the same engine. Never free them;
//    copy into std::string if the text must survive.
//  - userdb_save/load/decay persist and age the learned dictionary.
//    save = 0 ok / 2 IO / -1 null(engine|path); load = 0 ok / 1 file
//    missing (non-fatal, engine keeps its state) / 2 IO / 3 format /
//    -1 null; decay = 0 ok / -1 null, half_life_secs == 0 is legal and
//    changes nothing. load only merges into the user dictionary — preedit,
//    commit and mode are untouched. Paths are NUL-terminated (raw bytes).
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
int32_t ior_engine_page_next(IorEngineC* engine);
int32_t ior_engine_page_prev(IorEngineC* engine);
uint32_t ior_engine_page_info(const IorEngineC* engine);
const char* ior_engine_select(IorEngineC* engine, uint32_t page_local_index);
uint32_t ior_engine_ascii_mode(const IorEngineC* engine);
uint32_t ior_engine_set_ascii_mode(IorEngineC* engine, uint32_t enable);
int32_t ior_engine_userdb_save(IorEngineC* engine, const char* path);
int32_t ior_engine_userdb_load(IorEngineC* engine, const char* path);
int32_t ior_engine_userdb_decay(IorEngineC* engine, uint32_t elapsed_secs,
                                uint32_t half_life_secs);
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

    // Persists the learned dictionary (snapshot == compaction). Public so
    // candidate words can persist after a mouse selection, exactly like
    // the keyboard commit paths. Failures are silent.
    void saveUserdb();

private:
    IorEngineC* raw_;
    // Shift-as-toggle tracking: a Shift press only flips the mode if no
    // other key was pressed while it was held (i.e. it was not used as a
    // modifier). The toggle itself runs on Shift RELEASE through
    // ior_engine_set_ascii_mode, so Shift presses are never forwarded to
    // ior_engine_process_key (which would toggle a second time).
    bool shiftDown_ = false;
    bool shiftUsed_ = false;
};

// Selectable candidate word; select() commits through ior_engine_select
// with the candidate's page-local index, so commit/learn logic stays in
// Rust and clicking works on every page (no synthetic digit keys).
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
