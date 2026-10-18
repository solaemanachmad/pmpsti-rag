import pytest

from worker import fetch


class FakeResp:
    def __init__(self, status=200, json_data=None, content=b"", headers=None, text=""):
        self.status_code, self._json, self.content = status, json_data, content
        self.headers, self.text = headers or {}, text

    def json(self):
        return self._json

    def raise_for_status(self):
        if self.status_code >= 400:
            raise RuntimeError(self.status_code)


class FakeSession:
    def __init__(self, responses):
        self.responses, self.calls = list(responses), []

    def get(self, url, params=None, headers=None, timeout=None, allow_redirects=True):
        self.calls.append((url, params, headers))
        return self.responses.pop(0)


def test_list_drive_folder_recurses_and_filters():
    s = FakeSession([
        FakeResp(json_data={"files": [
            {"id": "f1", "name": "B.pdf", "mimeType": "application/pdf", "md5Checksum": "m1",
             "webViewLink": "https://drive.google.com/file/d/f1/view"},
            {"id": "sub", "name": "Arsip", "mimeType": "application/vnd.google-apps.folder"},
            {"id": "x", "name": "foto.jpg", "mimeType": "image/jpeg"},
        ]}),
        FakeResp(json_data={"files": [
            {"id": "d1", "name": "A doc", "mimeType": "application/vnd.google-apps.document",
             "modifiedTime": "2026-10-01T00:00:00Z"},
        ]}),
    ])
    files = fetch.list_drive_folder(s, "root", "KEY")
    assert [f.name for f in files] == ["A doc", "B.pdf"]
    assert files[0].remote_rev == "2026-10-01T00:00:00Z" and files[0].citation_url.endswith("/d1/view")
    assert files[1].remote_rev == "m1"
    assert "'root' in parents" in s.calls[0][1]["q"] and s.calls[0][1]["key"] == "KEY"


def test_drive_permission_error_is_explained():
    s = FakeSession([FakeResp(status=404, text="File not found")])
    with pytest.raises(RuntimeError, match="Anyone with the link"):
        fetch.get_drive_file(s, "abc", "KEY")


def test_web_pdf_conditional_and_validation():
    pdf = b"%PDF-1.7 fake"
    s = FakeSession([FakeResp(content=pdf, headers={"ETag": '"v1"', "Content-Type": "application/pdf"})])
    d = fetch.download_web_pdf(s, "https://x.ac.id/a.pdf")
    assert d.remote_rev == 'etag:"v1"' and d.sha256 == fetch.sha256(pdf)

    s = FakeSession([FakeResp(status=304)])
    d = fetch.download_web_pdf(s, "https://x.ac.id/a.pdf", 'etag:"v1"')
    assert d.data is None and s.calls[0][2]["If-None-Match"] == '"v1"'

    s = FakeSession([FakeResp(content=b"<html>login</html>", headers={"Content-Type": "text/html"})])
    with pytest.raises(RuntimeError, match="Bukan file PDF"):
        fetch.download_web_pdf(s, "https://x.ac.id/a.pdf")

    s = FakeSession([FakeResp(status=404)])
    with pytest.raises(RuntimeError, match="HTTP 404"):
        fetch.download_web_pdf(s, "https://x.ac.id/a.pdf")


def test_web_file_name():
    f = fetch.web_file("https://jteti.ugm.ac.id/wp-content/uploads/Buku%20Panduan%202022.pdf?x=1")
    assert f.name == "Buku Panduan 2022.pdf" and f.external_id == f.citation_url
