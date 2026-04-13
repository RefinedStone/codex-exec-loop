# CKA 네트워크 최소 핵심 슬라이드 설계안

## 문서 목적

- 대상 강의: [cka-network-90min-lecture-plan.md](/Users/newin_mac/Documents/dev/akra/codex-exec-loop-worktrees/docs-native-platform-presentation-cka-network-90min/cka-network-90min-lecture-plan.md)
- 목표: 90분 강의안을 실제 슬라이드 제작 단위로 분해해, 발표 자료 제작과 강사 리허설에 바로 사용할 수 있게 한다.
- 원칙: 슬라이드는 이론 설명보다 "시험에서 어디부터 확인할지"를 남기는 순서로 배치한다.
- HTML 드래프트: [slides/cka-network-90min-draft/index.html](/Users/newin_mac/Documents/dev/akra/codex-exec-loop-worktrees/docs-native-platform-presentation-cka-network-90min/slides/cka-network-90min-draft/index.html)

## 전체 슬라이드 원칙

- 한 슬라이드에는 핵심 문장 하나만 남긴다.
- 구현 원리보다 문제 풀이 순서가 먼저 보이게 한다.
- Pod, Service, DNS, Ingress, NetworkPolicy는 각각 따로 설명하되 마지막에는 반드시 한 흐름으로 다시 묶는다.
- 데모 슬라이드는 "무엇을 보여 줄지"와 "학생이 무엇을 관찰해야 하는지"를 같이 적는다.
- 명령어 슬라이드는 나열보다 "어떤 상황에서 어떤 명령을 먼저 쓰는가" 기준으로 묶는다.

## 권장 슬라이드 수

- 총 26장
- 설명 슬라이드 18장
- 데모 슬라이드 4장
- 드릴 슬라이드 3장
- 마무리 슬라이드 1장

## 시간 배분 요약

| 구간 | 슬라이드 | 시간 |
| --- | --- | --- |
| 오프닝 | 1-3 | 5분 |
| 네트워크 기초 | 4-6 | 10분 |
| 쿠버네티스 네트워크 기본 구조 | 7-9 | 12분 |
| Service | 10-14 | 15분 |
| DNS | 15-17 | 10분 |
| Ingress | 18-19 | 10분 |
| NetworkPolicy | 20-22 | 13분 |
| 트러블슈팅 드릴 | 23-25 | 11분 |
| 마무리 | 26 | 4분 |

## 슬라이드별 구성안

### 1. 제목 슬라이드

- 제목: `CKA 합격을 위한 쿠버네티스 네트워크 최소 핵심`
- 목표: 오늘 강의의 범위와 톤을 한 줄로 고정한다.
- 화면 구성:
  - 메인 타이틀
  - 서브카피: `시험에 필요한 최소 네트워크만 빠르게 정리`
- 발표 포인트:
  - 오늘은 네트워크 심화가 아니라 CKA 합격용 최소 핵심만 다룬다.
  - 모든 내용은 시험과 트러블슈팅 순서에 맞춰 설명한다.
- 예상 시간: 1분

### 2. 오늘 끝나고 할 수 있어야 하는 것

- 제목: `오늘 끝나고 바로 할 수 있어야 하는 것`
- 목표: 수강생 기대치를 실전 기준으로 맞춘다.
- 화면 구성:
  - `Pod -> Service -> DNS -> Ingress/Policy` 흐름 화살표
  - 오른쪽에 4개 학습 성과
- 발표 포인트:
  - Service 타입 구분
  - DNS 이름 해석 이해
  - Ingress와 NetworkPolicy 역할 구분
  - 통신 장애 1차 분류
- 예상 시간: 2분

### 3. 오늘 과감히 빼는 것

- 제목: `오늘은 이걸 하지 않습니다`
- 목표: 불필요한 심화 기대를 미리 정리한다.
- 화면 구성:
  - 좌측 `다루는 것`
  - 우측 `안 다루는 것`
- 발표 포인트:
  - OSI 7계층, BGP, VXLAN, kube-proxy 내부 구현은 이번 범위가 아니다.
  - 대신 시험에서 실제로 푸는 방식만 남긴다.
- 예상 시간: 2분

### 4. 네트워크 최소 용어 1

- 제목: `IP, Port, DNS만 먼저 맞춥니다`
- 목표: 이후 설명에 필요한 최소 용어를 통일한다.
- 화면 구성:
  - `example.com:443` 예시를 이름, 주소, 포트로 분해
  - 하단에 DNS 한 줄 설명
- 발표 포인트:
  - IP는 대상 주소, Port는 출입구다.
  - DNS는 이름을 IP로 바꿔 준다.
- 예상 시간: 4분

### 5. 네트워크 최소 용어 2

- 제목: `CIDR은 이 정도만 알면 충분합니다`
- 목표: Service CIDR, Pod CIDR 표기를 읽게 만든다.
- 화면 구성:
  - `10.96.0.0/12`, `192.168.0.0/24` 예시
  - 하단에 `시험에서는 계산보다 대역 읽기가 중요`
- 발표 포인트:
  - 대역 표기를 보고 "어느 범위인가"만 읽으면 충분하다.
  - 복잡한 subnetting 계산은 이번 범위가 아니다.
- 예상 시간: 3분

### 6. 네트워크 최소 용어 3

- 제목: `TCP/UDP는 왜 거의 TCP만 생각해도 되는가`
- 목표: 용어 설명을 길게 끌지 않고 쿠버네티스로 넘어간다.
- 화면 구성:
  - TCP와 UDP 한 줄 비교
  - 하단에 `CKA 네트워크 실습은 대부분 TCP 중심`
- 발표 포인트:
  - 시험 문제에서는 Service, DNS, HTTP 연결 확인이 많아서 TCP 기준으로 이해해도 된다.
- 예상 시간: 3분

### 7. 쿠버네티스 네트워크 3원칙

- 제목: `쿠버네티스 네트워크는 세 문장으로 시작합니다`
- 목표: 전체 흐름의 공통 기반을 만든다.
- 화면 구성:
  - 세 문장을 크게 배치
  - 각 문장 옆에 작은 아이콘
- 발표 포인트:
  - 모든 Pod는 IP를 가진다.
  - Pod 간 통신이 가능해야 한다.
  - 고정 진입점은 Pod가 아니라 Service가 맡는다.
- 예상 시간: 4분

### 8. Pod IP와 Node 관점

- 제목: `Pod는 IP를 가지지만 오래 믿을 주소는 아닙니다`
- 목표: Pod IP와 Service를 분리해서 이해시킨다.
- 화면 구성:
  - `kubectl get pods -o wide` 예시 캡처
  - Pod IP, Node 컬럼 강조
- 발표 포인트:
  - Pod는 재생성되면 IP가 바뀔 수 있다.
  - 시험에서 Pod 직접 확인과 Service 경유 확인을 구분해야 한다.
- 예상 시간: 4분

### 9. 한 장 그림

- 제목: `Pod, Service, DNS를 한 장으로 보면`
- 목표: 이후 섹션의 참조 그림을 하나 만든다.
- 화면 구성:
  - 두 개 노드와 여러 Pod
  - 앞단 Service
  - 옆에 CoreDNS
- 발표 포인트:
  - 사용자는 보통 Service 이름으로 붙는다.
  - CoreDNS가 이름을 IP로 바꾸고, Service가 실제 Pod로 연결한다.
- 예상 시간: 4분

### 10. Service가 필요한 이유

- 제목: `왜 Pod IP로 직접 붙지 않고 Service를 쓰는가`
- 목표: Service 존재 이유를 먼저 이해시킨다.
- 화면 구성:
  - 좌측: Pod IP 직접 접근의 문제
  - 우측: Service를 둔 뒤의 안정성
- 발표 포인트:
  - Pod는 바뀐다.
  - Service는 고정된 진입점과 로드밸런싱을 제공한다.
- 예상 시간: 3분

### 11. Service 타입 비교

- 제목: `Service 네 가지는 이 차이만 기억합니다`
- 목표: 시험용 최소 구분 기준을 남긴다.
- 화면 구성:
  - `ClusterIP`, `NodePort`, `LoadBalancer`, `Headless` 4칸 비교표
- 발표 포인트:
  - 내부 기본값은 `ClusterIP`
  - 외부 포트 노출은 `NodePort`
  - 클라우드 외부 LB 연결은 `LoadBalancer`
  - 가상 IP 없이 Pod 레코드를 직접 드러내면 `Headless`
- 예상 시간: 4분

### 12. selector와 endpoints

- 제목: `Service는 selector로 Pod를 고르고 endpoints로 드러납니다`
- 목표: Service 진단의 핵심 연결고리를 남긴다.
- 화면 구성:
  - `Service -> selector -> Pod -> endpoints` 흐름도
  - 하단에 `Service가 안 되면 endpoints부터`
- 발표 포인트:
  - selector가 틀리면 endpoints가 비게 된다.
  - endpoints가 있으면 뒤쪽 Pod와 포트 문제로 좁혀진다.
- 예상 시간: 4분

### 13. Service 데모 슬라이드

- 제목: `데모: selector가 틀리면 무엇이 보이는가`
- 목표: 수강생이 화면에서 봐야 할 포인트를 미리 고정한다.
- 화면 구성:
  - 왼쪽: 실행 명령
  - 오른쪽: 기대 관찰 포인트
- 보여 줄 것:
  - `kubectl get svc`
  - `kubectl get endpoints`
  - `kubectl describe svc <name>`
- 발표 포인트:
  - Service는 존재하지만 endpoints가 없을 수 있다.
  - 이 경우 가장 먼저 selector와 Pod label을 본다.
- 예상 시간: 2분 설명 + 3분 데모

### 14. Service 문제 풀이 순서

- 제목: `Pod to Service 실패 시 보는 순서`
- 목표: 진단 순서를 암기 문장으로 남긴다.
- 화면 구성:
  - 번호 1~5 체크리스트
  - `Service -> selector -> endpoints -> targetPort -> Pod 상태`
- 발표 포인트:
  - 이 순서를 외우면 대부분의 Service 문제를 빠르게 줄일 수 있다.
- 예상 시간: 2분

### 15. CoreDNS와 이름 해석

- 제목: `DNS는 이름이 IP로 바뀌는가만 먼저 봅니다`
- 목표: DNS 문제를 다른 연결 문제와 구분하게 한다.
- 화면 구성:
  - `client -> service name -> CoreDNS -> IP`
  - 하단에 `이름이 IP로 바뀌면 DNS는 통과`
- 발표 포인트:
  - DNS 문제인지 확인할 때는 조회 결과가 가장 먼저다.
  - 조회가 되면 Service 또는 앱 문제로 넘어간다.
- 예상 시간: 4분

### 16. namespace와 FQDN

- 제목: `왜 어떤 서비스는 짧은 이름으로 되고 어떤 것은 안 되는가`
- 목표: namespace/FQDN 오해를 줄인다.
- 화면 구성:
  - 같은 namespace 예시
  - 다른 namespace 예시 `service.namespace.svc.cluster.local`
- 발표 포인트:
  - 같은 namespace는 짧은 이름으로 되는 경우가 많다.
  - 다른 namespace는 FQDN이 필요할 수 있다.
- 예상 시간: 3분

### 17. DNS 데모 슬라이드

- 제목: `데모: 일반 Service와 headless Service의 DNS 차이`
- 목표: DNS 결과 차이를 시각적으로 보여 준다.
- 화면 구성:
  - `nslookup` 또는 `dig` 결과 두 개 비교
  - 일반 Service와 headless Service를 색으로 구분
- 보여 줄 것:
  - 일반 Service는 하나의 Service IP
  - headless Service는 여러 Pod IP 가능
- 발표 포인트:
  - DNS 조회 결과만 봐도 Service 종류를 어느 정도 추론할 수 있다.
- 예상 시간: 2분 설명 + 3분 데모

### 18. Ingress 한 장 요약

- 제목: `Ingress는 밖에서 들어오는 규칙입니다`
- 목표: Service와 Ingress의 역할을 분리한다.
- 화면 구성:
  - 외부 사용자 -> Ingress -> Service -> Pod 흐름도
  - `host`와 `path` 강조
- 발표 포인트:
  - Ingress는 보통 Service를 뒤에 둔다.
  - Ingress 문제도 결국 Service와 Pod까지 같이 봐야 한다.
- 예상 시간: 5분

### 19. Ingress YAML 읽기

- 제목: `Ingress YAML은 네 줄만 먼저 봅니다`
- 목표: 시험 YAML 읽기 기준을 남긴다.
- 화면 구성:
  - 예제 YAML 일부
  - `host`, `path`, `service.name`, `service.port` 강조
- 발표 포인트:
  - 문제에서 이미 Controller가 준비된 경우가 많다.
  - 리소스 생성보다 rule 해석과 backend 연결이 더 중요하다.
- 예상 시간: 5분

### 20. NetworkPolicy 사고방식

- 제목: `NetworkPolicy는 기본적으로 이렇게 읽습니다`
- 목표: 가장 흔한 오해를 먼저 제거한다.
- 화면 구성:
  - `정책 대상 Pod`와 `허용된 출발지/목적지` 박스
  - 하단에 `클러스터 전체 방화벽이 아님`
- 발표 포인트:
  - 가장 먼저 볼 것은 어떤 Pod에 정책이 붙는지다.
  - 그 다음 ingress인지 egress인지 본다.
- 예상 시간: 4분

### 21. NetworkPolicy YAML 읽기

- 제목: `podSelector, policyTypes, from/to만 읽어도 절반은 풉니다`
- 목표: YAML 해석의 최소 기준을 남긴다.
- 화면 구성:
  - 예제 YAML 일부
  - `podSelector`, `policyTypes`, `from`, `namespaceSelector`, `podSelector` 강조
- 발표 포인트:
  - 허용 규칙을 읽는 문제인지
  - 누가 막히는지 판별하는 문제인지 구분한다.
- 예상 시간: 4분

### 22. NetworkPolicy 데모 슬라이드

- 제목: `데모: backend는 왜 되고 client는 왜 안 되는가`
- 목표: 정책 적용 전후의 체감 차이를 보여 준다.
- 화면 구성:
  - 세 Pod 관계도: `frontend`, `backend`, `client`
  - 정책 적용 전후 통신 결과 비교
- 보여 줄 것:
  - policy 없을 때 통신 가능
  - backend를 선택하는 ingress policy 추가
  - frontend만 성공, client 실패
- 발표 포인트:
  - NetworkPolicy는 누구를 막을지가 아니라 누구에게 정책이 적용됐는지를 먼저 읽는다.
- 예상 시간: 2분 설명 + 3분 데모

### 23. 트러블슈팅 드릴 1

- 제목: `드릴: Pod to Pod가 안 되면`
- 목표: 네트워크 문제를 첫 질문으로 줄이는 훈련을 한다.
- 화면 구성:
  - 짧은 장애 문장
  - 오른쪽에 빈 체크리스트
- 발표 포인트:
  - Running 여부
  - Pod IP 확인
  - 직접 IP 접속
  - 앱 포트와 readiness 확인
- 예상 시간: 4분

### 24. 트러블슈팅 드릴 2

- 제목: `드릴: Pod to Service와 DNS가 안 되면`
- 목표: Service 문제와 DNS 문제를 분리하게 만든다.
- 화면 구성:
  - 두 개 상황 카드
  - 각 카드 아래 첫 명령 2개를 적게 하는 공간
- 발표 포인트:
  - Service는 endpoints부터
  - DNS는 `nslookup`부터
- 예상 시간: 4분

### 25. 트러블슈팅 드릴 3

- 제목: `드릴: NetworkPolicy 적용 후 왜 막혔는가`
- 목표: policy 해석 순서를 반복한다.
- 화면 구성:
  - policy YAML 짧은 예시
  - 오른쪽에 해석 순서 4단계
- 발표 포인트:
  - 어떤 Pod에 정책이 붙는가
  - ingress인가 egress인가
  - label이 맞는가
  - namespace가 맞는가
- 예상 시간: 3분

### 26. 시험 직전 체크리스트

- 제목: `시험 직전 이 다섯 문장만 남기면 됩니다`
- 목표: 강의 종료 후 암기 문장을 남긴다.
- 화면 구성:
  - 5개 핵심 문장
  - 하단에 명령어 치트시트 축약본
- 발표 포인트:
  - Pod는 변한다. Service는 고정 진입점이다.
  - Service 문제는 selector와 endpoints부터 본다.
  - DNS 문제는 이름이 IP로 바뀌는지부터 본다.
  - Ingress 문제도 결국 뒤의 Service와 Pod를 함께 본다.
  - NetworkPolicy는 선택된 Pod에 허용 규칙을 붙이는 방식으로 읽는다.
- 예상 시간: 4분

## 제작 시 바로 넣을 시각 자료

- 두 노드와 여러 Pod, Service, CoreDNS가 함께 보이는 전체 구조도 1장
- Service 타입 4분할 비교표 1장
- selector와 endpoints 흐름도 1장
- 일반 Service와 headless Service의 DNS 응답 비교 캡처 1장
- Ingress 요청 흐름도 1장
- NetworkPolicy 적용 전후 통신 변화 다이어그램 1장

## 데모 슬라이드에 붙일 명령어 후보

- `kubectl get pods -o wide`
- `kubectl get svc`
- `kubectl get endpoints`
- `kubectl describe svc <name>`
- `kubectl exec -it <pod> -- nslookup <service>`
- `kubectl exec -it <pod> -- dig <service>`
- `kubectl get networkpolicy`
- `kubectl describe networkpolicy <name>`
- `kubectl exec -it <pod> -- curl http://<service>:<port>`
- `kubectl exec -it <pod> -- wget -qO- http://<service>:<port>`

## 발표자 메모

- Service 섹션과 DNS 섹션은 연결해서 설명하되, 학생 머릿속에서는 반드시 분리되게 해야 한다.
- Ingress는 화려하게 다루지 말고 "Service 앞단 규칙" 정도로만 정리한다.
- NetworkPolicy는 YAML 전체를 읽게 하지 말고 선택 대상과 허용 조건만 읽게 훈련한다.
- 트러블슈팅 드릴은 정답 설명보다 첫 명령을 묻는 방식으로 운영한다.
