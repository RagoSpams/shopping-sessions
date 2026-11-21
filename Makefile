IMAGE ?= shopping-sessions:latest
CPUS  ?= 8
MEM   ?= 24g
# The scorer's platform. PLATFORM=linux/arm64 builds and runs natively on an Apple-silicon Mac, for fast local loops.
PLATFORM ?= linux/amd64
DEV   ?= dev
# In the monorepo the fixtures live two levels up; the published starter has its own copy.
FIXTURES ?= $(if $(wildcard fixtures),fixtures,../../fixtures)
export IMAGE CPUS MEM PLATFORM

.PHONY: build fixtures dev run-dev check-dev

## Build the image for the scorer's platform, linux/amd64 (emulated on Apple silicon)
build:
	docker buildx build --platform $(PLATFORM) --load -t $(IMAGE) .

## Run the image on all 36 edge-case fixtures and diff against the expected tables
fixtures: build
	python3 scripts/run_fixtures.py $(FIXTURES) -- scripts/docker_run.sh

## Make the 1M-event dev set with the published generator (needs `generate` on PATH)
dev:
	generate --seed 1 --events 1000000 --clicks 250000 --output $(DEV)

## Run on the dev set with scorer flags and time it
run-dev: build
	rm -rf out && time scripts/docker_run.sh $(DEV) out

## Compare your dev output with the published dev answer (dev-answer.zip, unzipped): make check-dev ANSWER=dev-answer
ANSWER ?= dev-answer
check-dev:
	python3 scripts/compare.py out $(ANSWER)

PY ?= $(HOME)/shopping/.venv/bin/python
DATA_ROOT ?= $(HOME)/shopping/data
GEN ?= $(HOME)/shopping/generator/target/release/generate
DEV_ANSWER ?= $(HOME)/shopping/dev-answer

.PHONY: fixtures-local dev-local run-dev-local check-dev-local gen-local

fixtures-local:
	@scratch=$$(mktemp -d) && \
	trap 'rm -rf -- "$$scratch"' EXIT && \
	SCRATCH_DIR="$$scratch" "$(PY)" -B scripts/run_fixtures.py fixtures -- "$(PY)" -B pipeline/pipeline.py

dev-local:
	@if [ ! -f "$(DATA_ROOT)/dev/manifest.json" ]; then \
		"$(GEN)" --seed 1 --events 1000000 --clicks 250000 --output "$(DATA_ROOT)/dev"; \
	fi

run-dev-local: dev-local
	rm -rf -- "$(DATA_ROOT)/dev-out" "$(DATA_ROOT)/dev-scratch"
	mkdir -p "$(DATA_ROOT)/dev-out" "$(DATA_ROOT)/dev-scratch"
	DATA_DIR="$(DATA_ROOT)/dev" OUTPUT_DIR="$(DATA_ROOT)/dev-out" SCRATCH_DIR="$(DATA_ROOT)/dev-scratch" \
		NCPU=8 MEM_GB=24 time -p "$(PY)" -B pipeline/pipeline.py

check-dev-local:
	"$(PY)" -B scripts/compare.py "$(DATA_ROOT)/dev-out" "$(DEV_ANSWER)"

gen-local:
	"$(GEN)" --seed "$(SEED)" --events "$(EVENTS)" --clicks "$(CLICKS)" \
		--output "$(DATA_ROOT)/s$(SEED)-$(EVENTS)"
