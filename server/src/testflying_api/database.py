from __future__ import annotations

from collections.abc import Generator
from pathlib import Path

from fastapi import Request
from sqlalchemy import Engine, create_engine
from sqlalchemy.engine import make_url
from sqlalchemy.orm import Session, sessionmaker
from sqlalchemy.pool import StaticPool

SessionFactory = sessionmaker[Session]


def create_engine_for_url(database_url: str) -> Engine:
    url = make_url(database_url)
    connect_args: dict[str, object] = {}
    kwargs: dict[str, object] = {}

    if url.drivername.startswith("sqlite"):
        connect_args["check_same_thread"] = False
        if url.database and url.database != ":memory:":
            Path(url.database).parent.mkdir(parents=True, exist_ok=True)
        if url.database == ":memory:":
            kwargs["poolclass"] = StaticPool

    return create_engine(database_url, connect_args=connect_args, **kwargs)


def create_session_factory(engine: Engine) -> SessionFactory:
    return sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)


def get_db_session(request: Request) -> Generator[Session]:
    session_factory: SessionFactory = request.app.state.session_factory
    session = session_factory()
    try:
        yield session
    finally:
        session.close()
