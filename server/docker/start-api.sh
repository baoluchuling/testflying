#!/bin/sh
set -e

if [ "${TESTFLYING_AUTO_MIGRATE:-1}" = "1" ]; then
  attempts="${TESTFLYING_AUTO_MIGRATE_ATTEMPTS:-10}"
  delay="${TESTFLYING_AUTO_MIGRATE_DELAY_SECONDS:-2}"
  count=1

  until alembic upgrade head; do
    if [ "$count" -ge "$attempts" ]; then
      echo "Database migration failed after ${attempts} attempts." >&2
      exit 1
    fi
    echo "Database migration failed; retrying in ${delay}s (${count}/${attempts})..." >&2
    count=$((count + 1))
    sleep "$delay"
  done
fi

exec uvicorn testflying_api.main:app --host 0.0.0.0 --port 8000
