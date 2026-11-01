# fcitx5 "ior" addon: thin C++ shell linked against the Rust engine.
# Written for fcitx5 5.1.x; see README.md in this directory.
cmake_minimum_required(VERSION 3.16)
project(fcitx5-ior CXX)

# Where the fcitx5-ior cargo package lives (drives `cargo build`), and the
# cargo workspace root that owns target/. Both are overridable on the cmake
# command line, e.g. -DIOR_WORKSPACE_DIR=/abs/path/to/ior.
set(IOR_RUST_DIR "${CMAKE_CURRENT_SOURCE_DIR}/.." CACHE PATH
    "Directory of the fcitx5-ior cargo package")
set(IOR_WORKSPACE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../.." CACHE PATH
    "Cargo workspace root (contains target/)")
set(IOR_RUST_STATICLIB "${IOR_WORKSPACE_DIR}/target/release/libior_engine.a")

find_package(Fcitx5Core REQUIRED)
find_package(Fcitx5Utils REQUIRED)

# Build the Rust side first (produces the staticlib linked below plus the
# cdylib used for `nm` checks). BYPRODUCTS because cargo is incremental.
add_custom_target(
  ior_rust ALL
  COMMAND cargo build --release -p fcitx5-ior
  WORKING_DIRECTORY "${IOR_RUST_DIR}"
  BYPRODUCTS "${IOR_RUST_STATICLIB}"
  COMMENT "Building ior Rust engine (libior_engine.a)"
  VERBATIM)

add_library(ior MODULE ioraddon.cpp)
add_dependencies(ior ior_rust)
target_link_libraries(
  ior PRIVATE Fcitx5::Core Fcitx5::Utils
  ${IOR_RUST_STATICLIB} pthread dl m c)
# fcitx5 addon modules are named lib<name>.so; metadata lives in
# <datadir>/fcitx5/addon (see e.g. wayland.conf: Library=libwayland).
set_target_properties(ior PROPERTIES CXX_STANDARD 17
                                      CXX_STANDARD_REQUIRED ON)

# Install dirs normally come from Fcitx5Core's cmake config; fall back to
# relocatable GNUInstallDirs paths so --prefix works (user install).
include(GNUInstallDirs)
if(NOT FCITX_INSTALL_ADDONDIR)
  set(FCITX_INSTALL_ADDONDIR "${CMAKE_INSTALL_LIBDIR}/fcitx5")
endif()
if(NOT FCITX_INSTALL_ADDONCFGDIR)
  set(FCITX_INSTALL_ADDONCFGDIR
      "${CMAKE_INSTALL_DATADIR}/fcitx5/addon")
endif()
if(NOT FCITX_INSTALL_INPUTMETHODDIR)
  set(FCITX_INSTALL_INPUTMETHODDIR
      "${CMAKE_INSTALL_DATADIR}/fcitx5/inputmethod")
endif()

install(TARGETS ior DESTINATION "${FCITX_INSTALL_ADDONDIR}")
install(FILES ior.conf DESTINATION "${FCITX_INSTALL_ADDONCFGDIR}")
install(FILES inputmethod/ior.conf DESTINATION "${FCITX_INSTALL_INPUTMETHODDIR}")

# Theme icon referenced by `Icon=ior` in inputmethod/ior.conf. The source
# tree keeps brand assets at <workspace>/assets so README and the addon share
# one file.
install(FILES "${IOR_WORKSPACE_DIR}/assets/ior.svg"
        DESTINATION "${CMAKE_INSTALL_DATADIR}/icons/hicolor/scalable/apps"
        RENAME ior.svg)
install(FILES "${IOR_WORKSPACE_DIR}/assets/ior-48.png"
        DESTINATION "${CMAKE_INSTALL_DATADIR}/icons/hicolor/48x48/apps"
        RENAME ior.png)
