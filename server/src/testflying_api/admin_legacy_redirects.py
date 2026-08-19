from __future__ import annotations


def admin_next_redirect_path(path: str) -> str | None:
    """Keep the temporary /admin-next entrypoint working after /admin becomes canonical."""
    if path == "/admin-next" or path == "/admin-next/":
        return "/admin"
    if path.startswith("/admin-next/"):
        return f"/admin{path.removeprefix('/admin-next')}"
    return None
