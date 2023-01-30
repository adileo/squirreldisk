# SquirrelDisk

<br>

<p align="center">
    <a href="https://github.com/adileo/squirreldisk"><img src="https://img.shields.io/github/v/release/adileo/squirreldisk?color=%23ff00a0&include_prereleases&label=version&sort=semver&style=flat-square"></a>
    &nbsp;
    <a href="https://github.com/adileo/squirreldisk"><img src="https://img.shields.io/badge/built_with-Rust-dca282.svg?style=flat-square"></a>
     &nbsp;
     <a href="https://discord.gg/Xp8QtMM65wk"><img src="https://img.shields.io/badge/Discord-%235865F2.svg?style=flat-square&logo=discord&logoColor=white"></a>
   
</p>

<div align="center">

[![Windows Support](https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white)](https://github.com/adileo/squirreldisk/releases) [![Ubuntu Support](https://img.shields.io/badge/Ubuntu-E95420?style=for-the-badge&logo=ubuntu&logoColor=white)](https://github.com/adileo/squirreldisk/releases) [![Arch Linux Support](https://img.shields.io/badge/Arch_Linux-1793D1?style=for-the-badge&logo=arch-linux&logoColor=white)](https://github.com/adileo/squirreldisk/releases) [![Windows Support](https://img.shields.io/badge/MACOS-adb8c5?style=for-the-badge&logo=macos&logoColor=white)](https://github.com/adileo/squirreldisk/releases)

</div>

![Screenshot](https://www.squirreldisk.com/images/demo2.gif)

## What's taking your hard disk space?

The easiest app you will ever use to detect huge files. Built with Rust + React (Tauri).

## Installation
Please note that the current version is not stable yet, and you may encounter various bugs, especially on Linux and MacOS.

### Windows
Just install with the .msi

Currently I'm still facing permissions issue in the sidecar binary embedding of PDU (parallel disk usage) for Linux/MacOS. Will try to fix soon in the meanwhile:
### Ubuntu
After installing the .deb file make sure to `sudo chmod 777 /usr/bin/pdu` and run squirreldisk with sudo 'sudo squirrel-disk'. 

### MacOS
Install the app from the .dmg, then `sudo chmod 777 /Applications/SquirrelDisk.app/Contents/MacOS/pdu`, then you can open the app with right click > Open.


## Disclaimer

This app was a project from 2 years ago built in Electron in 2 days, I decided to port it to Tauri to achieve better performances and to make it Open Source. Yay.

The code is still spaghetti and needs a lot of refactoring.

## Bug Reporting

If you find any bugs, please report it by submitting an issue on our [issue page](https://github.com/adileo/squirreldisk/issues) with a detailed explanation. Giving some screenshots would also be very helpful.

## Feature Request

You can also submit a feature request on our [issue page](https://github.com/adileo/squirreldisk/issues) or [discussions]((https://github.com/adileo/squirreldisk/discussions) and we will try to implement it as soon as possible.

## Contributions

- [Join our Discord Server](https://discord.gg/Xp8QtMM65w)

## Credits

- [parallel-disk-usage](https://github.com/KSXGitHub/parallel-disk-usage)
- [tauri](https://github.com/tauri-apps/tauri)
