from __future__ import annotations

from sqlalchemy.orm import Session

from testflying_api.schema import App, Build
from testflying_api.seed import seed_demo_catalog


def test_seed_demo_catalog_creates_apps_and_builds(db_session: Session) -> None:
    seed_demo_catalog(db_session)

    assert db_session.query(App).count() >= 3
    assert db_session.query(Build).count() >= 3
