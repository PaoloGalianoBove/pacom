SCENARIO ?= rtt
TOPOLOGY ?= single-host
ENV_FILE := pacom/docker/env/$(SCENARIO).env
COMPOSE_FILE := pacom/docker/compose.$(TOPOLOGY).yaml
PROFILE_ARGS := $(if $(filter mqtt-bridge,$(SCENARIO)),--profile mqtt,)
COMPOSE := docker compose $(PROFILE_ARGS) --env-file $(ENV_FILE) -f $(COMPOSE_FILE)
SERVICE ?= app-b

.PHONY: docker-build docker-up docker-down docker-ps docker-logs docker-output docker-attach docker-config docker-rebuild

docker-config:
	@test -f "$(ENV_FILE)" || (echo "Unknown scenario: $(SCENARIO)" >&2; exit 2)
	@test -f "$(COMPOSE_FILE)" || (echo "Unknown topology: $(TOPOLOGY)" >&2; exit 2)
	@mkdir -p results
	$(COMPOSE) config --quiet

docker-build: docker-config
	$(COMPOSE) build

docker-up: docker-config
	$(COMPOSE) up --detach --build --remove-orphans

docker-down:
	$(COMPOSE) down --volumes --remove-orphans

docker-ps:
	$(COMPOSE) ps

docker-logs:
	$(COMPOSE) logs --follow

docker-output:
	$(COMPOSE) logs $(SERVICE)

docker-attach:
	@container_id="$$($(COMPOSE) ps --quiet $(SERVICE))"; \
	if [ -z "$$container_id" ]; then \
		echo "Service '$(SERVICE)' is not running; use 'make docker-output SCENARIO=$(SCENARIO) TOPOLOGY=$(TOPOLOGY) SERVICE=$(SERVICE)'" >&2; \
		exit 2; \
	fi; \
	docker attach "$$container_id"

docker-rebuild: docker-config
	$(COMPOSE) build --no-cache