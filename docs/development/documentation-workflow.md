---
title: Documentation Workflow
description: Normative guidelines for documentation spines, review gates, and docs-first engineering
category: development
audience: developers
document_type: guide
status: accepted
website_publish: true
sidebar_order: 31
---

# Documentation Workflow

> Status: **accepted**. This document defines the authoring standards, structural spines, and quality gates for documentation within Phodopus.

---

## 1. Documentation-First Principle

Phodopus strictly follows a **documentation-first** engineering discipline:

1. **Architecture & Contract First**: No code implementation, dependency addition, or interface refactoring begins without an accepted specification or architectural document.
2. **Synchronized Evidence**: Documentation claims must match reality. Planned features, experimental ideas, or draft proposals must never be framed as implemented behavior.
3. **English-Only Requirement**: All documentation, code comments, commit messages, and issues are authored exclusively in technical English.

---

## 2. Docs Self-Containment

The Phodopus documentation corpus must remain completely self-contained:

- **Zero Scratch Citations**: Documents must never cite private discussion tickets, research log numbers, or temporary scratch paths.
- **Stand-Alone Comprehension**: An engineer must be able to comprehend and implement every technical feature relying solely on this corpus and the public Rust/Lua documentation.

---

## 3. Structural Spines per Document Type

Every document must adhere to the standardized section spine for its `document_type`:

### 3.1 Specification Spine (`document_type: specification`)

1. Frontmatter (YAML: `title`, `description`, `category: specifications`, `document_type: specification`, `status`)
2. Title & Status Banner
3. Purpose and Scope (In scope / Out of scope)
4. Normative Sources This Specification Must Not Weaken
5. Terminology
6. Technical Body (Architecture, Data structures, Protocol sequences)
7. Security Review / Considerations
8. Verification Plan
9. Alternatives Considered
10. Affected Contracts
11. Acceptance Criteria

### 3.2 Architecture Spine (`document_type: architecture`)

1. Frontmatter (YAML: `title`, `description`, `category: architecture`, `document_type: architecture`, `status`)
2. Title & Status Banner
3. System Vision & Context
4. Component Architecture & Structural Diagrams
5. Invariants & Guarantees
6. Sequence / Trampoline Execution Flows
7. Cross-Cutting Concerns (Memory, Concurrency, Error Handling)

### 3.3 Index Spine (`document_type: index`)

1. Frontmatter (YAML: `title`, `description`, `category`, `document_type: index`, `status`)
2. Navigation Scope & Purpose
3. Directory Content Table (Name, Type, Status, Description)

---

## 4. Review & Quality Gates

During pull request and peer review, documentation is evaluated as first-class code. A reviewer returns `NEEDS-FIX` if:

- A document violates the defined section spine.
- Non-English content is found.
- Unimplemented interfaces are described as existing facts without status qualification.
- External research record numbers or private chat references are embedded.
