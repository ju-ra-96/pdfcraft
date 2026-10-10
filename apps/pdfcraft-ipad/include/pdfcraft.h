// The PdfCraft engine's C interface for the iPad app (apps/pdfcraft-ipad/src/lib.rs).
//
// One JSON request in, one JSON response out:
//   request:  {"tool": "<tool name>", "args": {...}}      (the tools of `pdfcraft-cli tools`)
//   response: {"ok": [{"json": {...}} | {"png": "<base64>", "width": W, "height": H}, ...]}
//             {"error": {"kind": "bad_request" | "unknown_tool" | "invalid_args" | "failed" | "internal",
//                        "message": "..."}}
// An engine is used by one thread at a time. Nothing here crashes on bad input.

#ifndef PDFCRAFT_H
#define PDFCRAFT_H

#include <stddef.h>
#include <stdint.h>

typedef struct Engine PdfCraftEngine;

typedef struct {
    uint8_t *ptr;
    size_t len;
} PdfCraftBuffer;

// A new engine whose tools read and write files only inside the folder `root` (UTF-8), or
// anywhere when `root` is NULL. NULL when the folder can't be used.
PdfCraftEngine *pdfcraft_engine_new(const char *root);

// Free an engine (NULL is ignored).
void pdfcraft_engine_free(PdfCraftEngine *engine);

// Run one request of `len` bytes. Free the result with pdfcraft_buffer_free.
PdfCraftBuffer pdfcraft_call(PdfCraftEngine *engine, const uint8_t *request, size_t len);

// Give back a buffer from pdfcraft_call.
void pdfcraft_buffer_free(PdfCraftBuffer buffer);

#endif
