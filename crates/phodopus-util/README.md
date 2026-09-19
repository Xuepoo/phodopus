# phodopus-util

Helper utilities for the `phodopus` pure-Rust Lua virtual machine.

This crate provides optional, ergonomic utilities that complement the core
`phodopus` library for host application integration.

## Features

- **Lifetime erasure and freeze validation**: Safely erases lifetimes from Rust values and performs runtime checks to ensure values are not accessed past their lifetime.
- **Serde integration**: Easy conversion between Rust data types and Lua values.
- **Userdata helpers**: Quick metatable construction and binding for host userdata types.
