# Idea: Artifact PATH Correlation for Carved File Naming

**Status:** Deferred — low practical value for the general case
**Captured:** 2026-03-20

---

## The Problem

When Carving extracts a file and there is no filesystem metadata (directory entry was
deleted or overwritten), the file gets an offset-based name like
`ferrite_exe_12345678.exe`.  These files are difficult to identify and are largely
useless without their original names and folder structure — an EXE placed in the wrong
directory won't run, and without a name it can't be triaged efficiently.

The Artifacts tab independently finds Windows PATH strings (e.g.
`C:\WINDOWS\system32\mstsc.exe`) by scanning raw bytes.  The question is whether those
path strings can be used to name anonymously carved files.

---

## Why the General Case Has Low Value

An EXE or DLL without its original name and correct folder structure does not function.
Renaming `ferrite_exe_12345678.exe` to `mstsc.exe` without placing it in
`C:\WINDOWS\system32\` doesn't recover usability.  For most binary formats the
recovered file is only useful as forensic evidence of existence, not as a runnable
artifact.  The naming hint therefore provides marginal incremental value beyond what
the byte offset itself already encodes.

---

## Mechanism (if ever implemented)

### Spatial Containment Matching

A carved file at byte offset `O` with size `S` physically contains any data at offsets
in `[O, O+S)`.  If an Artifacts PATH hit at offset `X` satisfies `O ≤ X < O+S`, the
path string is embedded *inside* that carved file.

```
for each carved file (offset=O, size=S, ext=E):
    candidates = [a for a in artifact_paths if O ≤ a.offset < O+S
                                            and a.path.extension == E]
    if len(candidates) == 1:
        rename_hint = candidates[0].path.filename   # high confidence
    elif candidates ranked by proximity to O:
        rename_hint = candidates[0].path.filename   # lower confidence
```

### High-Signal File Types (worth implementing if scoped narrowly)

| Format | Why reliable |
|---|---|
| **Windows Shortcut (.lnk)** | Target path is at a fixed offset (~0x4C); always refers to the linked file, not to the LNK itself. Extension mismatch between carved `.lnk` and target ext is expected and fine — gives the target identity. |
| **Prefetch (.pf)** | File header contains the executable name; the PF filename is canonically `<EXECNAME>-<HASH>.pf`. Parsing the header gives a rename with high confidence. |
| **PE Debug Directory** | `IMAGE_DEBUG_TYPE_CODEVIEW` record contains the PDB path, which usually matches the EXE/DLL name. Low false-positive rate if only the PDB path (not all imported DLL paths) is used. Requires a small PE parser on the extracted file, not on the raw artifact output. |

### Why Generic EXE/DLL Matching Is Noisy

A registry hive, browser cache, crash dump, or page file all contain hundreds of
unrelated PATH strings.  Matching any PATH artifact inside an EXE-carved region would
produce many false candidates (imported DLL names, INI file paths, resource strings).
The 1-to-1 confidence required for a reliable rename is rarely achievable.

---

## Implementation Sketch (if scoped to LNK / PF)

1. After a carving extraction completes, for hits with extension `lnk` or `pf`:
   - Read the first 512 bytes of the extracted file
   - Parse the LNK shell item or PF header to extract the embedded name
   - Rename the carved file to the recovered name (with a `[recovered]` suffix to flag
     it as a heuristic rename, not a confirmed filesystem name)
2. For `exe`/`dll`: query the Artifacts artifact index for PATH hits inside the file's
   byte range; filter to the single PATH that uses the PDB naming pattern
   (`<name>.pdb`); strip `.pdb` to get the module name.

### Shared State Required

- `ArtifactHit` list (currently in `ArtifactsState`) needs to be accessible from the
  Carving extraction path.
- Easiest approach: store a completed artifact scan result in `Arc<Vec<ArtifactHit>>`
  on `App`, passed into `CarvingState::filename_for_hit()` alongside the metadata index.
- Artifact hits must be sorted by `byte_offset` for binary-search range queries.

---

## Decision

Deferred.  The cost/benefit is too low for the general case — anonymously named
executables without their folder structure are not recoverable to a working state.
The LNK and PF cases are genuinely useful but narrow enough that they can be
implemented as a focused post-extraction pass when the need arises.
