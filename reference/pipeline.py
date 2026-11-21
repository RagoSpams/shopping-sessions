#!/usr/bin/env python3
"""Shopping Sessions reference implementation."""

from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import UTC, datetime
from itertools import islice
import json
import os
from pathlib import Path
import re

import pyarrow as pa
import pyarrow.parquet as pq

DAY_MS = 86_400_000
LOOKBACK_MS = 7 * DAY_MS
SESSION_GAP_MS = 1_800_000
MAX_PRICE_CENTS = 10_000_000
EPOCH = datetime(1970, 1, 1, tzinfo=UTC)
ORGANIC = b"organic"

ASCII_DIGITS = re.compile(rb"[0-9]+")
CALENDAR_TIMESTAMP = re.compile(
    rb"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
    rb"(?:Z|[+-]([0-9]{2}):([0-9]{2}))"
)

USER_SCHEMA = pa.schema([
    ("user_id", pa.string()),
    ("day", pa.date32()),
    ("sessions", pa.int64()),
    ("events", pa.int64()),
    ("revenue_cents", pa.int64()),
    ("top_category", pa.string()),
])
CAMPAIGN_SCHEMA = pa.schema([
    ("campaign_id", pa.string()),
    ("channel", pa.string()),
    ("day", pa.date32()),
    ("sessions", pa.int64()),
    ("purchasing_sessions", pa.int64()),
    ("revenue_cents", pa.int64()),
    ("assisted_sessions", pa.int64()),
])


def read_csv(path, field_count):
    # E20: LF alone separates lines, so retain a final record at EOF even
    # without LF. L3: skip only the first physical line. Any BOM there goes
    # with the header, and a BOM in a data field is part of its UTF-8 value.
    with path.open("rb") as source:
        next(source, None)
        for line in source:
            line = line.removesuffix(b"\n")
            if not line:
                continue
            fields = line.split(b",")
            if len(fields) != field_count:
                continue

            row = []
            for field in fields:
                if field.startswith(b'"'):
                    if len(field) < 2 or not field.endswith(b'"'):
                        break
                    field = field[1:-1]
                if b'"' in field:
                    break
                row.append(field or None)
            else:
                yield row


def read_parts(directory, field_count):
    for path in directory.glob("*.csv"):
        yield from read_csv(path, field_count)


def bounded_uint(value, maximum):
    if value is None or maximum < 0 or not ASCII_DIGITS.fullmatch(value):
        return None
    # Count significant digits before converting, so arbitrarily long inputs
    # with or without leading zeros cannot hit Python's int conversion limit.
    digits = value.lstrip(b"0")
    if len(digits) > len(str(maximum)):
        return None
    number = int(digits or b"0")
    return number if number <= maximum else None


def calendar_ms(value):
    match = CALENDAR_TIMESTAMP.fullmatch(value)
    if match is None:
        return None
    hours, minutes = match.groups()
    if hours is not None and (int(hours) > 23 or int(minutes) > 59):
        return None
    try:
        delta = datetime.fromisoformat(value.decode("ascii")) - EPOCH
    except ValueError:
        return None
    return delta.days * DAY_MS + delta.seconds * 1_000


def windowed_ms(value, start, end):
    if value is None:
        return None
    if ASCII_DIGITS.fullmatch(value):
        timestamp = bounded_uint(value, end - 1)
    else:
        timestamp = calendar_ms(value)
    if timestamp is not None and start <= timestamp < end:
        return timestamp
    return None


def read_person_names(directory):
    parents = {}
    sizes = {}

    def find(user):
        while parents[user] != user:
            parents[user] = parents[parents[user]]
            user = parents[user]
        return user

    for first, second in read_parts(directory, 2):
        if first is None or second is None:
            continue
        for user in (first, second):
            if user not in parents:
                parents[user] = user
                sizes[user] = 1
        left, right = find(first), find(second)
        if left == right:
            continue
        if sizes[left] < sizes[right]:
            left, right = right, left
        parents[right] = left
        sizes[left] += sizes.pop(right)

    roots = {user: find(user) for user in parents}
    minimums = {}
    for user, root in roots.items():
        if root not in minimums or user < minimums[root]:
            minimums[root] = user
    return {user: minimums[root] for user, root in roots.items()}


def read_events(directory, people, products, window_start, window_end):
    earliest = {}
    for event_id, user, timestamp, ingest, kind, product, price in read_parts(directory, 7):
        if event_id is None:
            continue
        previous = earliest.get(event_id)
        # Ingest is always valid F1, for which byte order is chronological.
        if previous is None or ingest < previous[0]:
            earliest[event_id] = (ingest, people.get(user, user), timestamp, kind, product, price)

    by_person = defaultdict(list)
    while earliest:
        _, (ingest, person, timestamp, kind, product, price) = earliest.popitem()
        if person is None or kind not in (b"view", b"cart", b"purchase") or product is None:
            continue
        timestamp = windowed_ms(timestamp, window_start, window_end)
        if timestamp is None:
            continue

        revenue = 0
        category = None
        if kind == b"purchase":
            revenue = bounded_uint(price, MAX_PRICE_CENTS)
            if not revenue:
                continue
            category = products.get(product, b"unknown")

        if calendar_ms(ingest) - timestamp <= DAY_MS:
            by_person[person].append((timestamp, revenue, category))
    return by_person


def read_clicks(directory, people, campaigns, window_start, window_end):
    seen = set()
    by_person = defaultdict(list)
    for click_id, user, campaign, timestamp in read_parts(directory, 4):
        if click_id is None or click_id in seen:
            continue
        seen.add(click_id)
        if user is None or campaign not in campaigns:
            continue
        timestamp = windowed_ms(timestamp, window_start - LOOKBACK_MS, window_end)
        if timestamp is not None:
            by_person[people.get(user, user)].append((timestamp, click_id, campaign))
    return by_person


def sessions_for(events):
    events.sort(key=lambda event: event[0])
    start = previous = events[0][0]
    count = revenue = 0
    categories = Counter()
    for timestamp, price, category in events:
        if timestamp - previous > SESSION_GAP_MS:
            yield start, count, revenue, categories
            start = timestamp
            count = revenue = 0
            categories = Counter()
        previous = timestamp
        count += 1
        if category is not None:
            revenue += price
            categories[category] += price
    yield start, count, revenue, categories


def attributed_sessions(events, clicks):
    # A sorted timestamp group enters and leaves the window as a unit.
    # Its first click has the smallest byte-wise ID for that timestamp.
    clicks.sort()
    left = right = 0
    active = Counter()
    credited = ORGANIC
    for session in sessions_for(events):
        start = session[0]
        while right < len(clicks) and clicks[right][0] <= start:
            timestamp, _, campaign = clicks[right]
            if right == 0 or timestamp != clicks[right - 1][0]:
                credited = campaign
            active[campaign] += 1
            right += 1
        while left < right and clicks[left][0] < start - LOOKBACK_MS:
            campaign = clicks[left][2]
            active[campaign] -= 1
            if active[campaign] == 0:
                del active[campaign]
            left += 1

        # The caller consumes the campaign keys before advancing this iterator.
        yield session, credited if active else ORGANIC, active.keys()


@dataclass(slots=True)
class CampaignDay:
    sessions: int = 0
    purchasing_sessions: int = 0
    revenue: int = 0
    assists: int = 0


def user_rows(events, clicks, campaign_days):
    while events:
        person, person_events = events.popitem()
        person_clicks = clicks.pop(person, [])
        user_id = person.decode("utf-8")
        day = None
        sessions = event_count = revenue = 0
        categories = Counter()
        for session, credited, assists in attributed_sessions(person_events, person_clicks):
            start, count, session_revenue, session_categories = session
            session_day = start // DAY_MS
            if session_day != day:
                if day is not None:
                    yield user_row(user_id, day, sessions, event_count, revenue, categories)
                day = session_day
                sessions = event_count = revenue = 0
                categories = Counter()
            sessions += 1
            event_count += count
            revenue += session_revenue
            categories.update(session_categories)

            credit = campaign_days[credited, day]
            credit.sessions += 1
            credit.purchasing_sessions += session_revenue > 0
            credit.revenue += session_revenue
            for campaign in assists:
                campaign_days[campaign, day].assists += 1
        yield user_row(user_id, day, sessions, event_count, revenue, categories)


def user_row(user_id, day, sessions, events, revenue, categories):
    top = min(categories, key=lambda category: (-categories[category], category), default=None)
    return user_id, day, sessions, events, revenue, top.decode("utf-8") if top is not None else None


def campaign_rows(campaign_days, campaigns):
    for (campaign, day), totals in campaign_days.items():
        channel = ORGANIC if campaign == ORGANIC else campaigns[campaign]
        yield (
            campaign.decode("utf-8"),
            channel.decode("utf-8"),
            day,
            totals.sessions,
            totals.purchasing_sessions,
            totals.revenue,
            totals.assists,
        )


def write_parquet(directory, schema, rows):
    directory.mkdir(parents=True, exist_ok=True)
    with pq.ParquetWriter(directory / "part-0.parquet", schema) as writer:
        while batch := list(islice(rows, 65_536)):
            columns = dict(zip(schema.names, zip(*batch)))
            writer.write_table(pa.Table.from_pydict(columns, schema=schema))


def main():
    data = Path(os.environ.get("DATA_DIR", "/data"))
    output = Path(os.environ.get("OUTPUT_DIR", "/output"))
    os.environ.setdefault("TMPDIR", os.environ.get("SCRATCH_DIR", "/scratch"))
    with (data / "manifest.json").open(encoding="utf-8") as source:
        manifest = json.load(source)
    window_start = calendar_ms(manifest["window_start"].encode("ascii"))
    window_end = calendar_ms(manifest["window_end"].encode("ascii"))
    people = read_person_names(data / "links")
    products = dict(read_csv(data / "products.csv", 2))
    campaigns = dict(read_csv(data / "campaigns.csv", 2))
    events = read_events(data / "events", people, products, window_start, window_end)
    clicks = read_clicks(data / "clicks", people, campaigns, window_start, window_end)

    campaign_days = defaultdict(CampaignDay)
    write_parquet(output / "user_daily", USER_SCHEMA, user_rows(events, clicks, campaign_days))
    write_parquet(
        output / "campaign_daily",
        CAMPAIGN_SCHEMA,
        campaign_rows(campaign_days, campaigns),
    )


if __name__ == "__main__":
    main()
