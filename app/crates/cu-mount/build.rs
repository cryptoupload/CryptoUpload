// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
    #[cfg(windows)]
    winfsp::build::winfsp_link_delayload();
}
