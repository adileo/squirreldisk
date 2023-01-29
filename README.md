# SquirrelDisk

<br>

<p align="center">
    <a href="https://github.com/adileo/squirreldisk"><img src="https://img.shields.io/github/v/release/adileo/squirreldisk?color=%23ff00a0&include_prereleases&label=version&sort=semver&style=flat-square"></a>
    &nbsp;
    <a href="https://github.com/adileo/squirreldisk"><img src="https://img.shields.io/badge/built_with-Rust-dca282.svg?style=flat-square"></a>
     &nbsp;
     <a href="https://discord.gg/Xp8QtMM65wk"><img src="https://img.shields.io/badge/Discord-%235865F2.svg?style=flat-square&logo=discord&logoColor=white"></a>
   
</p>

![Screenshot](https://www.squirreldisk.com/images/demo2.gif)

## What's taking your hard disk space?

The easiest app you will ever use to detect huge files. Built with Rust + React (Tauri).

## Installation
### Windows
Just install with the .msi

Currently I'm still facing permissions issue in the sidecar binary embedding of PDU (parallel disk usage) for Linux/MacOS. Will try to fix soon in the meanwhile:
### Ubuntu
After installing the .deb file make sure to `sudo chmod 777 /usr/bin/pdu` and run squirreldisk with sudo 'sudo squirrel-disk'. 

### MacOS
Install the app from the .dmg, then `sudo chmod 777 /Applications/SquirrelDisk.app/Contents/MacOS`, then you can open the app with right click > Open.


## Disclaimer

This app was a project from 2 years ago built in Electron in 2 days, I decided to port it to Tauri to achieve better performances and to make it Open Source. Yay.

The code is still spaghetti and needs a lot of refactoring.

## Contributions

- [Join our Discord Server](https://discord.gg/Xp8QtMM65w)

## Credits

- [parallel-disk-usage](https://github.com/KSXGitHub/parallel-disk-usage)
- [tauri](https://github.com/tauri-apps/tauri)
