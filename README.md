# Souls Client

This is rough working copy of a dll I created which was used in a few segments of [Kaizo Colosseum: Souls Edition 2026](https://tiltify.com/@colosseum/kaizo-souls-2026), fundraising for Wings for Life. It connects to a server via websocket to receive commands for game interactions. The actual game interactions will be split out into other libraries in the future.

Some functionality is based on [vswarte/fromsoftware-rs](https://github.com/vswarte/fromsoftware-rs) and [Dasaav-dsv/erfps2](https://github.com/Dasaav-dsv/erfps2) which are licensed under Apache 2.0 and MIT licenses, and [ThomasJClark/GlintScript](https://github.com/ThomasJClark/GlintScript) which is licensed under MIT License. Currently this dll depends on the fork in [thefifthmatt/fromsoftware-rs](https://github.com/thefifthmatt/fromsoftware-rs), with the eventual goal of contributing functionality back upstream and reducing bespoke implementations in this dll.
