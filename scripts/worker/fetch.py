"""Fetch files from Google Drive (public, API key) and from website PDF links.

Every function returns plain data so it can be unit tested without network
(see tests/test_fetch.py, which feeds fake HTTP responses).
"""
from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass, field

import requests

DRIVE_API = "https://www.googleapis.com/drive/v3/files"
FOLDER_MIME = "application/vnd.google-apps.folder"
PDF_MIME = "application/pdf"
# Google-native files that can be exported to PDF
EXPORTABLE = {
    "application/vnd.google-apps.document",
    "application/vnd.google-apps.presentation",
}
TIMEOUT = 120
MAX_BYTES = 60 * 1024 * 1024  # guidebooks are a few MB; refuse anything absurd
USER_AGENT = "pmpsti-rag-worker/1.0 (+https://pmpsti-rag.vercel.app)"


@dataclass
class RemoteFile:
    external_id: str          # Drive file id, or the URL for web PDFs
    name: str
    citation_url: str         # link shown to students
    remote_rev: str = ""      # md5Checksum / modifiedTime / ETag / Last-Modified
    mime_type: str = PDF_MIME
    extra: dict = field(default_factory=dict)


@dataclass
class Download:
    data: bytes | None        # None = unchanged (HTTP 304)
    remote_rev: str
    sha256: str = ""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def drive_view_url(file_id: str) -> str:
    return f"https://drive.google.com/file/d/{file_id}/view"


# ── Google Drive ──────────────────────────────────────────────────────────────

def _drive_get(session: requests.Session, url: str, params: dict) -> requests.Response:
    resp = session.get(url, params=params, timeout=TIMEOUT)
    if resp.status_code == 403 and "accessNotConfigured" in resp.text:
        raise RuntimeError("Drive API belum diaktifkan di project Google Cloud untuk GOOGLE_API_KEY")
    if resp.status_code in (403, 404):
        raise RuntimeError(
            f"Drive menolak akses ({resp.status_code}). Pastikan file/folder dibagikan "
            "'Anyone with the link can view'."
        )
    resp.raise_for_status()
    return resp


def _drive_file(meta: dict) -> RemoteFile | None:
    mime = meta.get("mimeType", "")
    if mime != PDF_MIME and mime not in EXPORTABLE:
        return None
    return RemoteFile(
        external_id=meta["id"],
        name=meta.get("name", meta["id"]),
        citation_url=meta.get("webViewLink") or drive_view_url(meta["id"]),
        # md5Checksum exists for binary files; Google Docs only have modifiedTime
        remote_rev=meta.get("md5Checksum") or meta.get("modifiedTime", ""),
        mime_type=mime,
    )


def list_drive_folder(session: requests.Session, folder_id: str, api_key: str,
                      depth: int = 0, max_depth: int = 3) -> list[RemoteFile]:
    """All PDFs (and exportable Google Docs) in a public folder, including subfolders."""
    files: list[RemoteFile] = []
    token = None
    while True:
        params = {
            "q": f"'{folder_id}' in parents and trashed = false",
            "fields": "nextPageToken, files(id, name, mimeType, md5Checksum, modifiedTime, webViewLink)",
            "pageSize": 200,
            "supportsAllDrives": "true",
            "includeItemsFromAllDrives": "true",
            "key": api_key,
        }
        if token:
            params["pageToken"] = token
        data = _drive_get(session, DRIVE_API, params).json()
        for meta in data.get("files", []):
            if meta.get("mimeType") == FOLDER_MIME:
                if depth < max_depth:
                    files.extend(list_drive_folder(session, meta["id"], api_key, depth + 1, max_depth))
                continue
            f = _drive_file(meta)
            if f:
                files.append(f)
        token = data.get("nextPageToken")
        if not token:
            return sorted(files, key=lambda f: f.name.lower())


def get_drive_file(session: requests.Session, file_id: str, api_key: str) -> RemoteFile:
    meta = _drive_get(session, f"{DRIVE_API}/{file_id}", {
        "fields": "id, name, mimeType, md5Checksum, modifiedTime, webViewLink",
        "supportsAllDrives": "true",
        "key": api_key,
    }).json()
    f = _drive_file(meta)
    if not f:
        raise RuntimeError(f"File Drive '{meta.get('name')}' bukan PDF/Google Doc ({meta.get('mimeType')})")
    return f


def download_drive_file(session: requests.Session, f: RemoteFile, api_key: str) -> Download:
    if f.mime_type in EXPORTABLE:
        url, params = f"{DRIVE_API}/{f.external_id}/export", {"mimeType": PDF_MIME, "key": api_key}
    else:
        url, params = f"{DRIVE_API}/{f.external_id}", {"alt": "media", "supportsAllDrives": "true", "key": api_key}
    data = _drive_get(session, url, params).content
    check_pdf(data)
    return Download(data=data, remote_rev=f.remote_rev, sha256=sha256(data))


# ── Website PDFs ──────────────────────────────────────────────────────────────

def _rev_from_headers(headers) -> str:
    if headers.get("ETag"):
        return f"etag:{headers['ETag']}"
    if headers.get("Last-Modified"):
        return f"lm:{headers['Last-Modified']}"
    return ""


def web_file(url: str) -> RemoteFile:
    name = re.sub(r"[?#].*$", "", url).rstrip("/").rsplit("/", 1)[-1] or url
    return RemoteFile(external_id=url, name=requests.utils.unquote(name), citation_url=url)


def download_web_pdf(session: requests.Session, url: str, previous_rev: str = "") -> Download:
    headers = {"User-Agent": USER_AGENT}
    if previous_rev.startswith("etag:"):
        headers["If-None-Match"] = previous_rev[5:]
    elif previous_rev.startswith("lm:"):
        headers["If-Modified-Since"] = previous_rev[3:]
    resp = session.get(url, headers=headers, timeout=TIMEOUT, allow_redirects=True)
    if resp.status_code == 304:
        return Download(data=None, remote_rev=previous_rev)
    if not 200 <= resp.status_code < 300:
        raise RuntimeError(f"HTTP {resp.status_code} untuk {url}")
    data = resp.content
    check_pdf(data, resp.headers.get("Content-Type", ""))
    return Download(data=data, remote_rev=_rev_from_headers(resp.headers), sha256=sha256(data))


def check_pdf(data: bytes, content_type: str = "") -> None:
    if len(data) > MAX_BYTES:
        raise RuntimeError(f"File terlalu besar ({len(data) // 1_048_576} MB)")
    if not data.lstrip()[:5].startswith(b"%PDF"):
        hint = f" (Content-Type: {content_type})" if content_type else ""
        raise RuntimeError(f"Bukan file PDF{hint}. Link harus langsung ke file .pdf")
