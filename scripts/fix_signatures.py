"""Apply bulk max_size and size_hint fixes to signatures.toml."""
import re, shutil

with open('config/signatures.toml', 'r', encoding='utf-8') as f:
    text = f.read()

shutil.copy('config/signatures.toml', 'config/signatures.toml.bak')

# Helper: number pattern that matches TOML underscore-separated integers
NUM = r'\d[\d_]*'

changes = []

# 1-2. AAC TOML key bug: already applied (size_hint_kind = "adts") — skip

# 3. WMV: 4 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"Windows Media Video / ASF"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 4. PCX: 50 MB -> 1 MB
changes.append((
    r'(name\s*=\s*"PCX Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>1_048_576'
))

# 5. FLV: 500 MB -> 50 MB  (min_hit_gap already added in prior run — skip that step)
changes.append((
    r'(name\s*=\s*"Flash Video \(FLV\)"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>52_428_800'
))

# 6. AIFF: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"AIFF Audio"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 7. WAV: 2 GB -> 200 MB  (size hint fields already added in prior run — skip)
changes.append((
    r'(name\s*=\s*"WAV Audio \(RIFF\)"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 8. AVI: 2 GB -> 200 MB  (size hint fields already added in prior run — skip)
changes.append((
    r'(name\s*=\s*"AVI Video \(RIFF\)"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 9. FLAC: 2 GB -> 200 MB  (already applied? check)
changes.append((
    r'(name\s*=\s*"FLAC Audio"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 10. PSD: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"Adobe Photoshop Document[^"]*"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 11. PSB: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"Adobe Photoshop Large Document[^"]*"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 12. WavPack: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"WavPack Audio"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 13. VHD: 64 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"VHD Virtual Hard Disk"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 14. VHDX: 64 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"VHDX Virtual Hard Disk"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 15. QCOW2: 64 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"QCOW2 Virtual Disk[^"]*"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 16. VMDK: 10 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"VMDK Disk Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 17. PST/OST: 20 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"Outlook PST/OST"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 18. WTV: ~4 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"Windows WTV Television Recording"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 19. XZ: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"XZ Compressed"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 20. BZip2: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"BZip2 Compressed"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 21. RealMedia: 2 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"RealMedia / RealVideo"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 22. LUKS: 2 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"LUKS Encrypted Disk Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 23. E01: 2 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"EnCase Evidence File[^"]*"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 24. PCAP LE: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"PCAP Network Capture \(little-endian\)"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 25. PCAP BE: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"PCAP Network Capture \(big-endian\)"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 26. Blender: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"Blender 3D File"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 27. InDesign: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"Adobe InDesign Document"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 28. DPX BE: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"DPX Film Image \(Big-Endian\)"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 29. DPX LE: 2 GB -> 200 MB
changes.append((
    r'(name\s*=\s*"DPX Film Image \(Little-Endian\)"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 30. VDI: 2 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"VirtualBox VDI Disk Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 31. AFF: 2 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"AFF Forensic Disk Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 32. HDF5: 2 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"HDF5 Scientific Data File"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 33. FITS: 2 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"FITS Astronomy Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 34. Parquet: 2 GB -> 500 MB
changes.append((
    r'(name\s*=\s*"Apache Parquet"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>524_288_000'
))

# 35. KDBX: 512 MB -> 10 MB
changes.append((
    r'(name\s*=\s*"KeePass 2[^"]*"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>10_485_760'
))

# 36. KDB: 512 MB -> 10 MB
changes.append((
    r'(name\s*=\s*"KeePass 1[^"]*"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>10_485_760'
))

# 37. DMP: 512 MB -> 64 MB
changes.append((
    r'(name\s*=\s*"Windows Minidump"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>67_108_864'
))

# 38. plist: 100 MB -> 10 MB
changes.append((
    r'(name\s*=\s*"Apple Binary Property List[^"]*"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>10_485_760'
))

# 39-45. pyc x7: 100 MB -> 1 MB each
for ver in ['3.6', '3.7', '3.8', '3.9', '3.10', '3.11', '3.12']:
    vesc = ver.replace('.', r'\.')
    changes.append((
        rf'(name\s*=\s*"Python Bytecode {vesc} \(\.pyc\)"[^\[]*?max_size\s*=\s*)' + NUM,
        r'\g<1>1_048_576'
    ))

# 46. WOFF: 50 MB -> 10 MB
changes.append((
    r'(name\s*=\s*"WOFF Web Font"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>10_485_760'
))

# 47. APE (use ASCII name match to avoid apostrophe issues)
changes.append((
    r'(name\s*=\s*"Monkey[^"]*Audio[^"]*"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 48. CDR: 500 MB -> 100 MB
changes.append((
    r'(name\s*=\s*"CorelDRAW Drawing"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>104_857_600'
))

# 49. DjVu: 200 MB -> 50 MB
changes.append((
    r'(name\s*=\s*"DjVu Document"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>52_428_800'
))

# 50. XCF: 500 MB -> 100 MB
changes.append((
    r'(name\s*=\s*"GIMP XCF Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>104_857_600'
))

# 51. OpenEXR: 500 MB -> 200 MB
changes.append((
    r'(name\s*=\s*"OpenEXR High Dynamic Range Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>209_715_200'
))

# 52. JPEG 2000: 500 MB -> 50 MB
changes.append((
    r'(name\s*=\s*"JPEG 2000 Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>52_428_800'
))

# 53. JAR: 500 MB -> 100 MB
changes.append((
    r'(name\s*=\s*"Java Archive \(JAR\)"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>104_857_600'
))

# 54. EVT (legacy): 100 MB -> 20 MB
changes.append((
    r'(name\s*=\s*"Windows Event Log \(EVT\)"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>20_971_520'
))

# 55. PAR2: 1 GB -> 100 MB
changes.append((
    r'(name\s*=\s*"Parchive PAR2 Recovery Set"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>104_857_600'
))

# 56. BPG: 50 MB -> 5 MB
changes.append((
    r'(name\s*=\s*"BPG Image"[^\[]*?max_size\s*=\s*)' + NUM,
    r'\g<1>5_242_880'
))

# Apply all changes
modified = text
matched = []
unmatched = []
for i, (pattern, replacement) in enumerate(changes):
    new = re.sub(pattern, replacement, modified, flags=re.DOTALL)
    if new == modified:
        unmatched.append(f'#{i+1}: {pattern[:80]}')
    else:
        matched.append(f'#{i+1}')
        modified = new

with open('config/signatures.toml', 'w', encoding='utf-8') as f:
    f.write(modified)

print(f'Applied {len(matched)} changes: {", ".join(matched)}')
if unmatched:
    print(f'NO MATCH for {len(unmatched)}:')
    for u in unmatched:
        print(f'  {u}')
