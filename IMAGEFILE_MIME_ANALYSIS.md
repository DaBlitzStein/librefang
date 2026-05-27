# ImageFile MIME Type Handling Analysis

## Executive Summary

**Critical Finding:** NONE of the LLM drivers in librefang validate or reject non-image MIME types sent via `ContentBlock::ImageFile`. All drivers accept any arbitrary MIME type string without validation, encoding it directly into the API request. This means:

- **OpenAI driver**: Will send `data:application/pdf;base64,...` to OpenAI's API (will fail at API layer)
- **Anthropic driver**: Will send ANY MIME type to Anthropic in `image` content blocks (will fail at API layer)
- **Gemini driver**: Will send ANY MIME type in `inlineData` with no validation (Gemini's API may be truly MIME-agnostic)
- **All drivers**: No pre-validation, no rejection, no filtering — pure pass-through

---

## Driver-by-Driver Analysis

### 1. OpenAI Driver (`openai.rs`)

**Lines: 408-424**

```rust
ContentBlock::ImageFile { media_type, path } => {
    match std::fs::read(path) {
        Ok(bytes) => {
            use base64::Engine;
            let data = base64::engine::general_purpose::STANDARD
                .encode(&bytes);
            parts.push(OaiContentPart::ImageUrl {
                image_url: OaiImageUrl {
                    url: format!("data:{media_type};base64,{data}"),
                },
            });
        }
        Err(e) => {
            warn!(path = %path, error = %e, "ImageFile missing, skipping");
        }
    }
}
```

**Data Format:**
- Creates a data URI: `data:{media_type};base64,{base64_encoded_bytes}`
- Example for PDF: `data:application/pdf;base64,JVBERi0xLjQ...`

**MIME Type Validation:**
- **NONE** — `media_type` is used directly from the input without any validation
- No check for `image/`, no filtering, no rejection
- Will send literally ANY string in the `media_type` field

**API Behavior:**
- OpenAI's API only accepts `image_url` with image data URIs (`image/jpeg`, `image/png`, `image/gif`, `image/webp`)
- A PDF data URI (`data:application/pdf;base64,...`) will be **rejected by OpenAI's API** with a 400 error
- The rejection happens at the API layer, NOT in the driver

**Size Limits:**
- No size validation in driver
- File is read entirely into memory and base64-encoded
- Base64 expansion: ~33% overhead
- OpenAI has undocumented per-image size limits (approximately 20 MB raw, 5-10 MB in practice for large PDFs)

---

### 2. Anthropic Driver (`anthropic.rs`)

**Lines: 752-768**

```rust
ContentBlock::ImageFile { media_type, path } => match std::fs::read(path) {
    Ok(bytes) => {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Some(ApiContentBlock::Image {
            source: ApiImageSource {
                source_type: "base64".to_string(),
                media_type: media_type.clone(),
                data,
            },
        })
    }
    Err(e) => {
        warn!(path = %path, error = %e, "ImageFile missing, skipping");
        None
    }
},
```

**Data Format:**
- Sends as `ApiContentBlock::Image` with type tag `"image"`
- Content block structure:
  ```json
  {
    "type": "image",
    "source": {
      "type": "base64",
      "media_type": "{media_type}",
      "data": "{base64_data}"
    }
  }
  ```

**MIME Type Validation:**
- **NONE** — `media_type` is used directly
- No validation, no filtering, no rejection

**API Behavior:**
- Anthropic's API has **separate content block types** for different media:
  - `image` — for images only (image/jpeg, image/png, image/gif, image/webp, image/webp)
  - `document` — for PDFs and other documents (application/pdf)
- Sending a PDF with type `"image"` will be **rejected by Anthropic's API**
- The API will return a 400/422 error for unsupported `media_type` in image blocks
- The rejection happens at the API layer, NOT in the driver

**Size Limits:**
- No size validation in driver
- Anthropic's documented limits: 5 MB per image
- No special handling for large files

---

### 3. Gemini Driver (`gemini.rs`)

**Lines: 313-327**

```rust
ContentBlock::ImageFile { media_type, path } => match std::fs::read(path) {
    Ok(bytes) => {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        parts.push(GeminiPart::InlineData {
            inline_data: GeminiInlineData {
                mime_type: media_type.clone(),
                data,
            },
        });
    }
    Err(e) => {
        warn!(path = %path, error = %e, "ImageFile missing, skipping");
    }
},
```

**Data Format:**
- Sends as `GeminiPart::InlineData` with `mime_type` field
- Content structure:
  ```json
  {
    "inlineData": {
      "mimeType": "{media_type}",
      "data": "{base64_data}"
    }
  }
  ```

**MIME Type Validation:**
- **NONE** — `media_type` is used directly
- No validation, no filtering, no rejection

**API Behavior:**
- **Gemini's API appears to be MIME-agnostic** for the `inlineData` field
- The API accepts any valid MIME type string in the `mimeType` field
- Gemini does NOT validate or reject non-image MIME types at the API layer
- **A PDF sent as `application/pdf` would be accepted by Gemini's API** (though the model wouldn't necessarily process it correctly as an image)

**Size Limits:**
- No size validation in driver
- Gemini's documented limits: 20 MB per file

**Key Difference:**
- Gemini's approach is fundamentally different from OpenAI/Anthropic
- It uses a generic `inlineData` container with an arbitrary `mimeType` field
- This design allows it to accept PDFs and other non-image formats without API rejection
- The model's understanding/processing of non-image formats is another question entirely

---

### 4. Other Drivers Handling ImageFile

**Claude Code Driver** (`claude_code.rs`, 3 references)
- Handles ImageFile by referencing the path directly
- No base64 encoding
- No MIME type handling
- Not an API driver

**Qwen Code Driver** (`qwen_code.rs`, 15 references)
- Handles ImageFile by referencing the path directly
- CLI-based, not API-based
- Whitelists parent directories for sandbox access
- No MIME type validation

**Vertex AI Driver** (`vertex_ai.rs`)
- Uses Gemini's API format (inherits behavior)
- Same as Gemini driver

---

## MIME Type Validation Summary

### Validation Checklist

| Driver | Validates MIME? | Rejects Non-Image? | Where Rejection Happens | Works with PDF? |
|--------|-----------------|-------------------|------------------------|-----------------|
| OpenAI | ❌ No | ✅ Yes (API level) | OpenAI API rejects it | ❌ No (API error) |
| Anthropic | ❌ No | ✅ Yes (API level) | Anthropic API rejects it | ❌ No (API error) |
| Gemini | ❌ No | ❌ No | API accepts it | ✅ Likely (API allows) |
| Claude Code | N/A | N/A | N/A | N/A |
| Qwen Code | N/A | N/A | N/A | N/A |

### Key Finding

**The Moonshot/Kimi driver (used by RPi bot) is handled via the OpenAI-compatible driver**, which means:
- PDFs sent as `ContentBlock::ImageFile` with `media_type="application/pdf"` will be passed through as `data:application/pdf;base64,...`
- The Moonshot API will **reject** this with a 400 error (it only accepts image data URIs)
- **The rejection is NOT at the driver level — it's at the API level**

---

## Size Limits

### Per-Driver Limits

**OpenAI:**
- No enforced size check in driver
- File size limited by available memory + base64 expansion
- API limit: ~20 MB raw (varies by model)
- Effective practical limit: 5-10 MB for most images

**Anthropic:**
- No enforced size check in driver
- File size limited by available memory + base64 expansion
- API limit: 5 MB per image
- Documented and strict

**Gemini:**
- No enforced size check in driver
- File size limited by available memory + base64 expansion
- API limit: 20 MB per file
- Most generous

**All drivers:**
- Read entire file into memory with `std::fs::read(path)`
- No streaming, no chunking, no validation
- Base64 encodes entire payload
- ~33% size overhead from base64 encoding

---

## Critical Gaps

1. **No pre-validation in any driver** — drivers trust that `media_type` is correct
2. **No content-type filtering** — any MIME type string is accepted
3. **No size limits enforced** — large files could cause memory exhaustion
4. **No API-specific validation** — each driver could check for supported types
5. **Error messages at API layer only** — user sees "API error" not "invalid file type"

---

## Recommendations

### For ImageFile with Non-Image MIME Types

1. **OpenAI / Moonshot (Kimi):**
   - Add validation in driver to reject `media_type` that doesn't start with `image/`
   - Fail early with clear error: "OpenAI only accepts image/* MIME types"
   - Do NOT send to API

2. **Anthropic:**
   - Check if `media_type` is in the image whitelist: `image/jpeg`, `image/png`, `image/gif`, `image/webp`
   - If `application/pdf`, create a `document` content block instead of `image` block
   - If unknown type, fail early with clear error

3. **Gemini:**
   - Can accept non-image MIME types (API allows them)
   - No validation needed at driver level
   - Document that Gemini may not understand non-image formats

### For Size Limits

- Add configurable size limits per driver
- Check file size before reading into memory
- Provide clear error messages when exceeded

---

## Testing Implications

To test sending a PDF via `ContentBlock::ImageFile`:

**OpenAI/Moonshot:**
```rust
ContentBlock::ImageFile {
    media_type: "application/pdf".to_string(),
    path: "/path/to/file.pdf".to_string(),
}
```
**Result:** Driver sends `data:application/pdf;base64,...` → OpenAI API rejects with 400 error

**Anthropic:**
```rust
ContentBlock::ImageFile {
    media_type: "application/pdf".to_string(),
    path: "/path/to/file.pdf".to_string(),
}
```
**Result:** Driver sends `image` block with `media_type: "application/pdf"` → Anthropic API rejects with 400 error

**Gemini:**
```rust
ContentBlock::ImageFile {
    media_type: "application/pdf".to_string(),
    path: "/path/to/file.pdf".to_string(),
}
```
**Result:** Driver sends `inlineData` with `mimeType: "application/pdf"` → Gemini API accepts it (model behavior undefined)

---

## Conclusion

None of the LLM drivers validate `ContentBlock::ImageFile` MIME types before sending to APIs. Validation (and rejection) happens entirely at the API layer. OpenAI and Anthropic will reject PDFs; Gemini will accept them but with undefined behavior. All drivers lack size validation and use memory-intensive base64 encoding with no streaming support.
