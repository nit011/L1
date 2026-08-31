# iac.terraform_cloud
#
# Illustrative AWS (eu-west-1) infrastructure for VMs that run the image
# built from `infra/Dockerfile` (iac.dockerfile).
#
# This is NOT a production-hardened deployment: no secrets manager, no
# TLS provisioning, no IAM least-privilege hardening, no security-group
# lockdown beyond a default VPC example. Do not `terraform apply` this
# against a real cloud account as part of this tier.
#
# Provider choice: AWS because it is the most common IaaS target for a
# single-cloud example. A second cloud is out of schema scope.

terraform {
  required_version = ">= 1.5.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
}

variable "region" {
  type        = string
  description = "AWS region (illustrative)."
  default     = "eu-west-1"
}

variable "instance_count" {
  type        = number
  description = "Validator VMs (match compose N=4)."
  default     = 4
}

provider "aws" {
  region = var.region
}

# Build context is the git root; Dockerfile path is infra/Dockerfile.
# Operators would `docker build -f infra/Dockerfile` and push to a registry;
# this example only records that contract, it does not push.
locals {
  dockerfile = "${path.module}/../Dockerfile"
}

resource "aws_instance" "l1_node" {
  count         = var.instance_count
  ami           = data.aws_ami.debian.id
  instance_type = "t3.medium"

  user_data = <<-EOT
    #!/bin/bash
    set -euo pipefail
    # Illustrative: install docker and build from iac.dockerfile.
    # Real deployments should pull a pre-built image, not compile on the VM.
    apt-get update
    apt-get install -y docker.io
    # Placeholder clone/build — replace with a registry pull in production.
    echo "build with ${local.dockerfile}"
  EOT

  tags = {
    Name    = "l1-node-${count.index}"
    Purpose = "illustrative-iac-not-production"
  }
}

data "aws_ami" "debian" {
  most_recent = true
  owners      = ["136693071363"] # Debian
  filter {
    name   = "name"
    values = ["debian-12-amd64-*"]
  }
}

output "dockerfile_path" {
  value       = local.dockerfile
  description = "Path to iac.dockerfile used to build node images."
}

output "instance_ids" {
  value       = aws_instance.l1_node[*].id
  description = "Illustrative instance ids (empty until apply — do not apply in this tier)."
}
