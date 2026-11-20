variable "IMAGE_REGISTRY" {
  default = "ghcr.io/ferrislabs"
}

variable "IMAGE_TAG" {
  default = "dev"
}

variable "EXTRA_TAG" {
  default = ""
}

variable "RSPIGOT_DIR" {
  default = "/opt/minecraft/dev/rspigot"
}

function "image_tags" {
  params = [name]
  result = concat(
    ["${IMAGE_REGISTRY}/${name}:${IMAGE_TAG}"],
    EXTRA_TAG != "" ? ["${IMAGE_REGISTRY}/${name}:${EXTRA_TAG}"] : []
  )
}

group "default" {
  targets = ["orchestrator", "velocity", "rspigot", "lobby", "sheepwars"]
}

target "_common" {
  dockerfile = "Dockerfile"
  context    = "."
}

target "orchestrator" {
  inherits = ["_common"]
  target   = "orchestrator"
  tags     = image_tags("ferriscraft-orchestrator")
}

target "velocity" {
  inherits = ["_common"]
  target   = "velocity"
  tags     = image_tags("ferriscraft-velocity")
}

target "rspigot" {
  inherits = ["_common"]
  target   = "rspigot"
  contexts = {
    rspigot-src = RSPIGOT_DIR
  }
  tags = image_tags("ferriscraft-rspigot")
}

target "lobby" {
  inherits = ["_common"]
  target   = "lobby"
  contexts = {
    rspigot-src = RSPIGOT_DIR
  }
  tags = image_tags("ferriscraft-lobby")
}

target "sheepwars" {
  inherits = ["_common"]
  target   = "sheepwars"
  contexts = {
    rspigot-src = RSPIGOT_DIR
  }
  tags = image_tags("ferriscraft-sheepwars-plugin")
}
