# Product

<!-- impeccable:product-schema 1 -->

## Platform

Native desktop on Windows, macOS and Linux; egui rather than web content.

## Users

osu! mappers editing hitsounds and mapping instrument notes to gameplay objects.

## Product Purpose

Edit, audition, save and generate map audio with the familiar DAW arrangement. The complete target includes the node graph, Keysound mode, instruments, mixer, stems and client link.

## Stack

Rust, egui/eframe, cpal, Symphonia and the module boundaries in the vault. The user delegated implementation approvals on 2026-11-17.

## Operating Context

The actual project specification is E:/Obsidian/Workspace/dawsu-rewrite. The original app stays available. Validation uses copied maps and never changes original sample banks or plug-in installations.

## Product Principles

- Real map data and working actions are the product.
- Preserve untouched map bytes and newer external edits.
- Keep the familiar arrangement and precise editing semantics.
- Report physical audio and manual proof separately from tests.

## Evidence on Hand

Existing format/domain/kernel code, fixed map fixtures, accepted vault designs and recorded spike results. User delegated decisions; no new user research or runtime verification is claimed by this file.
