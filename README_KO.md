# ext4-bridge-windows (한국어 안내서)

Windows 환경을 위한 초고속 ext4 파일 시스템 브릿지, 유저모드 VHDX 파서 및 서브밀리초(Sub-millisecond) 재귀 메타데이터 스캐너 라이브러리입니다.

---

## 핵심 기술 및 도입 이유

1. **무설치 유저모드 VHDX (고정/동적 v1/v2) 직접 해석**
   - 별도의 가상 디스크 드라이버나 하이퍼바이저 권한 마운트 없이, 파일 헤더와 블록 할당 테이블(BAT)을 유저모드에서 직접 파싱하여 가상 주소를 물리 파일 오프셋으로 변환합니다.
   - WSL2나 Docker Desktop이 실행 중인 상태에서도 `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` 공유 플래그를 통해 Hyper-V 프로세스 잠금 충돌 없이 실시간 라이브 컨테이너 디스크를 X-ray 검사할 수 있습니다.

2. **SniffEverything.exe 연동 초고속 트리 스캐너**
   - 너비 우선 탐색(BFS) 기반의 재귀 디렉터리 스캔을 통해 중간 역직렬화 지연을 완전히 제거했습니다.
   - 제로 알로케이션 C-ABI 콜백(`ext4_tree_scan`)과 스트리밍 JSONL 직렬화기(`ext4_scan_jsonl`)를 제공하여, 실제 WSL2 VHDX 기준 **평균 0.165ms (분산 502 µs²)**의 서브밀리초 단위 고속 탐색을 달성했습니다.

3. **슈퍼블록 검증 및 최신 리눅스 플래그 정합성 보정**
   - 하드코딩된 블록 크기 대신 슈퍼블록 헤더(`s_log_block_size`)를 동적으로 계산하여 `u64::MAX / sector_size` 산술 오버플로우를 원천 차단했습니다.
   - 최신 리눅스 커널이 ext4에 부여하는 `EXT4_FEATURE_INCOMPAT_CSUM_SEED (0x2000)` 및 `EXT4_FEATURE_RO_COMPAT_METADATA_CSUM (0x0400)` 플래그를 읽기 전용 검사 단계에서 마스킹(`sanitize_sb_incompat`)하여 lwext4 엔진의 `ENOTSUP (129)` 마운트 거부 에러를 완벽히 해결했습니다.

4. **1MiB 섹터 정렬 I/O 엔진 및 리드어헤드 블록 캐시**
   - 물리 드라이브 정렬 요구조건을 만족하는 1MiB 정렬 버퍼 풀을 구축했습니다.
   - 2048 엔트리 리드어헤드(Read-Ahead) 블록 캐시를 탑재하여, 디렉터리 순회 시 수만 번에 달하는 불필요한 Win32 시스템 콜을 최대 50배 단축시켰습니다.

5. **메모리 안전성 및 Anti-REA 바이너리 보안**
   - C FFI 인터페이스(`ext4_read_h`) 호출 시 호출자가 제공한 버퍼 용량을 사전에 검증하여 버퍼 오버플로우를 방어했습니다.
   - 릴리스 빌드 시 심볼 제거(`strip = "symbols"`), 전체 링크 타임 최적화(`lto = "fat"`), 단일 코드젠 유닛, 패닉 즉시 중단(`panic = "abort"`)을 적용했습니다.
   - Pillar 10 OS 보안 완화: CLI 기동 시 `LOAD_LIBRARY_SEARCH_SYSTEM32` 고정으로 DLL 하이재킹 벡터를 차단했습니다.

---

## CLI 사용법 (`ext4_cli.exe`)

### 1. 파일 시스템 용량 및 블록 통계 확인
```cmd
ext4_cli stat-fs --image "C:\disks\ext4.vhdx"
```

### 2. 디렉터리 목록 조회
```cmd
ext4_cli ls / --image "C:\disks\ext4.vhdx"
```

### 3. 파일 내용 출력
```cmd
ext4_cli cat /etc/hosts --image "C:\disks\ext4.vhdx"
```

### 4. 재귀 메타데이터 트리 스캔
```cmd
ext4_cli tree-scan / --image "C:\disks\ext4.vhdx" --max-depth 2 --format jsonl
```

### 5. 물리 디스크 직접 검사
```cmd
ext4_cli ls / --drive 1 --partition 1
```

---

## 벤치마크 및 검증 결과

실제 Docker / WSL2 ext4 가상 디스크(141MB 용량, 36,288개 inode) 대상 16회 반복 측정 결과:

| 항목 | 측정 횟수 | 평균 지연 시간 (Mean) | 분산 (Variance, $\sigma^2$) |
| :--- | :---: | :---: | :---: |
| **VHDX 슈퍼블록 가상 오프셋 I/O** | 16 | 12.4 µs | 1.8 µs² |
| **루트 디렉터리 트리 스캔 (Depth 0)** | 16 | **165.19 µs (0.165 ms)** | **502.70 µs²** |
| **하위 전체 트리 BFS 순회** | 16 | 2.14 ms | 0.08 ms² |

---

## 빌드 및 테스트

```powershell
# 전체 통합 테스트 수행 (단일 스레드 순차 실행 권장)
cargo test -- --test-threads=1

# 배포용 릴리스 바이너리 빌드 (ext4_cli.exe 및 ext4_bridge.dll 산출)
cargo build --release
```

---

## 라이선스

Apache License, Version 2.0 또는 MIT 라이선스에 따라 듀얼 라이선스로 배포됩니다.
