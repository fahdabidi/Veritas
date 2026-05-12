package com.veritas.gbn.mobile

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Typeface
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.View
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import com.veritas.gbn.mobile.evidence.AppEventLog
import com.veritas.gbn.mobile.evidence.DeviceContext
import com.veritas.gbn.mobile.evidence.EvidenceBundleResult
import com.veritas.gbn.mobile.evidence.EvidenceBundleWriter
import com.veritas.gbn.mobile.evidence.S3EvidenceUploader
import com.veritas.gbn.mobile.model.CreatorAction
import com.veritas.gbn.mobile.model.CreatorActionCatalog
import com.veritas.gbn.mobile.model.EvidenceUploadConfig
import com.veritas.gbn.mobile.model.HostSeedGuard
import com.veritas.gbn.mobile.model.JsonText
import com.veritas.gbn.mobile.model.RunProfileConfig
import com.veritas.gbn.mobile.model.S3GrantQrAssembler
import com.veritas.gbn.mobile.runtime.RuntimeConfigFactory
import com.veritas.gbn.mobile.runtime.MobileCreatorRuntime
import com.veritas.gbn.mobile.service.CreatorForegroundService
import java.io.File

class MainActivity : Activity() {
    private lateinit var status: TextView
    private lateinit var eventLog: AppEventLog
    private lateinit var profileSpinner: Spinner
    private lateinit var runProfileInput: EditText
    private lateinit var hostSeedInput: EditText
    private lateinit var chainFilterInput: EditText
    private lateinit var uploadGrantInput: EditText
    private lateinit var scrollView: ScrollView
    private lateinit var output: TextView

    private var runtime: MobileCreatorRuntime? = null
    private var selectedProfile = RunProfileConfig.PROFILE_OFFLINE_TEST
    private var runProfile = RunProfileConfig.default(selectedProfile)
    private var lastHostSeedPayload: String? = null
    private var lastUploadResult: String = "{}"
    private var lastBootstrapResult: String = "{}"
    private var lastEvidence: EvidenceBundleResult? = null
    private val s3GrantQrAssembler = S3GrantQrAssembler()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        eventLog = AppEventLog(File(cacheDir, "app-events/events.jsonl"))
        requestPhase3Permissions()
        setContentView(buildUi())
        refreshStatus("App initialized")
    }

    override fun onDestroy() {
        runtime?.close()
        stopService(Intent(this, CreatorForegroundService::class.java))
        super.onDestroy()
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (resultCode != RESULT_OK) return
        val payload = data?.getStringExtra("SCAN_RESULT").orEmpty()
        if (payload.isBlank()) return
        when (requestCode) {
            REQUEST_HOST_QR_SCAN -> {
                hostSeedInput.setText(payload)
                previewHostSeed()
            }
            REQUEST_S3_GRANT_QR_SCAN -> importS3GrantQrPayload(payload)
        }
    }

    private fun buildUi(): View {
        scrollView = ScrollView(this).apply {
            setTag("MainScroll")
        }
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(24, 24, 24, 24)
        }
        scrollView.addView(root)

        root.addView(title("GBN Mobile Creator Validation"))
        status = body("Status pending")
        root.addView(status)

        addRuntimeScreen(root)
        addNetworkProfileScreen(root)
        addCreatorStateScreen(root)
        addCreatorActionsScreen(root)
        addBootstrapScreen(root)
        addUploadScreen(root)
        addEventsScreen(root)
        addEvidenceScreen(root)
        addResetScreen(root)

        output = body("Operation output appears here.")
        output.setTag("OperationOutput")
        root.addView(section("Output"))
        root.addView(output)
        return scrollView
    }

    private fun addRuntimeScreen(root: LinearLayout) {
        root.addView(section("Runtime"))
        root.addView(button("StartRuntime", "Start Runtime") { startRuntime() })
        root.addView(button("StopRuntime", "Stop Runtime") { stopRuntime() })
        root.addView(body("App build: ${BuildConfig.VERSION_NAME}\nABI: ${Build.SUPPORTED_ABIS.firstOrNull() ?: "unknown"}\nState path: ${stateDir().absolutePath}"))
    }

    private fun addNetworkProfileScreen(root: LinearLayout) {
        root.addView(section("Network Profile"))
        profileSpinner = Spinner(this).apply {
            adapter = ArrayAdapter(
                this@MainActivity,
                android.R.layout.simple_spinner_dropdown_item,
                listOf(
                    RunProfileConfig.PROFILE_OFFLINE_TEST,
                    RunProfileConfig.PROFILE_LOCAL_K8S_PUBLIC,
                    RunProfileConfig.PROFILE_HYBRID,
                ),
            )
            setTag("NetworkProfileSelector")
            onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                    selectedProfile = parent?.getItemAtPosition(position).toString()
                    runProfile = RunProfileConfig.default(selectedProfile)
                    refreshStatus("Selected $selectedProfile")
                }

                override fun onNothingSelected(parent: AdapterView<*>?) = Unit
            }
        }
        root.addView(profileSpinner)
        runProfileInput = multiLine(
            RunProfileConfig.default(RunProfileConfig.PROFILE_LOCAL_K8S_PUBLIC).rawJson,
            "RunProfileConfigInput",
        )
        root.addView(runProfileInput)
        root.addView(button("ImportEndpointConfig", "Import Run Profile") {
            runCatching {
                runProfile = RunProfileConfig.parse(runProfileInput.text.toString())
                selectedProfile = runProfile.profile
                eventLog.appendEvent("run_profile_imported", nextChainId("profile"), runProfile.profile)
                show("Imported run profile ${runProfile.runId}; profile=${runProfile.profile}")
            }.onFailure { showError(it) }
        })
    }

    private fun addCreatorStateScreen(root: LinearLayout) {
        root.addView(section("Creator State"))
        root.addView(button("ShowNodeMetadata", "Show Node Metadata") { callRuntime("ShowNodeMetadata") { it.nodeMetadata() } })
        root.addView(button("DumpLocalDht", "Dump Local DHT") { callRuntime("DumpLocalDht") { it.localDht() } })
        root.addView(button("DumpNodeState", "Dump Node State") {
            callRuntime("DumpNodeState") { runtime ->
                runtime.nodeMetadata() + "\n" + runtime.localDht()
            }
        })
        root.addView(button("RuntimeMetrics", "Runtime Metrics") {
            val events = eventLog.readText().lineSequence().filter { it.isNotBlank() }.count()
            show("runtime_running=${runtime != null}\napp_events=$events\nlast_upload=${lastUploadResult.take(240)}")
        })
    }

    private fun addCreatorActionsScreen(root: LinearLayout) {
        root.addView(section("Creator Actions"))
        CreatorActionCatalog.actions.forEach { action ->
            root.addView(actionButton(action))
        }
    }

    private fun addBootstrapScreen(root: LinearLayout) {
        root.addView(section("Bootstrap"))
        hostSeedInput = multiLine(sampleHostSeedPayload(), "HostCreatorDHTQRPayload")
        root.addView(hostSeedInput)
        root.addView(button("HostCreatorDHTQRReader", "HostCreator DHT QR Reader") {
            eventLog.appendAction(findAction("HostCreatorDHTQRReader"), nextChainId("qr"), "external_scanner_requested")
            val intent = Intent("com.google.zxing.client.android.SCAN").putExtra("SCAN_MODE", "QR_CODE_MODE")
            runCatching { startActivityForResult(intent, REQUEST_HOST_QR_SCAN) }
                .onFailure { show("No external QR scanner found in emulator. Paste or file-import the QR payload above.") }
        })
        root.addView(button("PreviewBootstrapDHTQR", "Preview Host Seed") {
            previewHostSeed()
        })
        root.addView(button("ImportHostCreatorDHTSeed", "Import Host Seed") {
            importHostSeed()
        })
        root.addView(button("BootstrapNewCreator", "Bootstrap New Creator") {
            bootstrapNewCreator()
        })
    }

    private fun addUploadScreen(root: LinearLayout) {
        root.addView(section("Upload"))
        root.addView(button("BuildUploadSession", "Build Synthetic Upload Session") {
            callRuntime("BuildUploadSession") { runtime ->
                val chainId = nextChainId("upload")
                val request = """{"chain_id":"$chainId","size_bytes":1048576,"chunk_size":65536,"sanitization_profile":"phase3-emulator"}"""
                runtime.buildSyntheticUploadSession(request).also { lastUploadResult = it }
            }
        })
        root.addView(button("SessionFrameSummary", "Session Frame Summary") {
            show("Last upload session:\n$lastUploadResult")
        })
        root.addView(button("SendDummy", "Send Dummy") {
            sendDummy()
        })
        root.addView(button("SendUpload", "Send Upload") {
            sendUpload()
        })
    }

    private fun addEventsScreen(root: LinearLayout) {
        root.addView(section("Events"))
        chainFilterInput = EditText(this).apply {
            hint = "ChainID filter"
            setText("mobile-upload-chain")
            setTag("ChainIdFilter")
        }
        root.addView(chainFilterInput)
        root.addView(button("RefreshEvents", "Refresh Events") {
            val chainId = chainFilterInput.text.toString().ifBlank { null }
            val filter = chainId?.let { """{"chain_id":"$it"}""" } ?: "{}"
            callRuntime("RefreshEvents") { it.traceEvents(filter) }
        })
    }

    private fun addEvidenceScreen(root: LinearLayout) {
        root.addView(section("Evidence"))
        uploadGrantInput = multiLine(
            """
            {
              "upload_mode": "s3_presigned_put",
              "bucket": "veritas-pass4-mobile-evidence",
              "object_key": "mobile-evidence/pass4-phase3-emulator/mobile-chain/mobile-bundle.zip",
              "presigned_put_url": "",
              "expires_at_ms": 0
            }
            """.trimIndent(),
            "S3EvidenceUploadGrant",
        )
        root.addView(uploadGrantInput)
        root.addView(body("ADB grant file: ${s3GrantFile().absolutePath}"))
        root.addView(button("ImportS3GrantFromDeviceFile", "Import S3 Grant From Device File") {
            importS3GrantFromDeviceFile()
        })
        root.addView(button("EvidenceGrantQRReader", "Evidence Grant QR Reader") {
            val intent = Intent("com.google.zxing.client.android.SCAN").putExtra("SCAN_MODE", "QR_CODE_MODE")
            runCatching { startActivityForResult(intent, REQUEST_S3_GRANT_QR_SCAN) }
                .onFailure { show("No external QR scanner found. Use Android Files/share import fallback, or emulator adb grant import.") }
        })
        root.addView(button("ExportEvidence", "Export Evidence") {
            exportEvidence()
        })
        root.addView(button("UploadEvidenceToS3", "Upload Evidence To S3") {
            uploadEvidenceToS3()
        })
    }

    private fun addResetScreen(root: LinearLayout) {
        root.addView(section("Reset"))
        root.addView(button("ResetCreatorState", "Reset Creator State") {
            AlertDialog.Builder(this)
                .setTitle("Reset creator state")
                .setMessage("Clear app-private creator state?")
                .setPositiveButton("Reset") { _, _ ->
                    callRuntime("ResetCreatorState") { it.resetState(nextChainId("reset")) }
                }
                .setNegativeButton("Cancel", null)
                .show()
        })
    }

    private fun actionButton(action: CreatorAction): Button =
        if (action.phase3Enabled) {
            button(action.buttonId, action.buttonId) {
                eventLog.appendAction(action, nextChainId(action.buttonId.lowercase()), "pressed")
                when (action.buttonId) {
                    "RefreshStatus" -> refreshStatus("Runtime running=${runtime != null}")
                    "RefreshState" -> callRuntime("RefreshState") { it.localDht() }
                    "DumpLocalDht" -> callRuntime("DumpLocalDht") { it.localDht() }
                    "DumpNodeState" -> callRuntime("DumpNodeState") { it.nodeMetadata() + "\n" + it.localDht() }
                    "RuntimeMetrics" -> show("events=${eventLog.readText().lines().size}; upload=${lastUploadResult.take(160)}")
                    "RefreshBridgeCatalog" -> callRuntime("RefreshBridgeCatalog") { it.refreshBridgeCatalog() }
                    "HostCreatorDHTQRReader" -> previewHostSeed()
                    "BootstrapNewCreator" -> bootstrapNewCreator()
                    "BuildUploadSession" -> callRuntime("BuildUploadSession") { runtime ->
                        val chainId = nextChainId("upload")
                        runtime.buildSyntheticUploadSession("""{"chain_id":"$chainId","size_bytes":4096,"chunk_size":1024,"sanitization_profile":"phase3-button"}""")
                    }
                    "SendDummy" -> sendDummy()
                    "SendUpload" -> sendUpload()
                    "SessionFrameSummary" -> show(lastUploadResult)
                    "ExportEvidence" -> exportEvidence()
                    "UploadEvidenceToS3" -> uploadEvidenceToS3()
                    "ResetCreatorState" -> callRuntime("ResetCreatorState") { it.resetState(nextChainId("reset")) }
                    else -> refreshStatus(action.buttonId)
                }
            }
        } else {
            disabledButton(action.buttonId, action.buttonId, action.disabledReason)
        }

    private fun startRuntime() {
        runCatching {
            runtime?.close()
            stateDir().mkdirs()
            evidenceDir().mkdirs()
            val config = RuntimeConfigFactory.build(
                stateDir = stateDir(),
                evidenceDir = evidenceDir(),
                creatorId = "android-${deviceId().take(12)}",
                networkProfile = selectedProfile,
                endpointConfigJson = runProfile.rawJson,
            )
            runtime = MobileCreatorRuntime(config)
            startForeground("runtime", "MobileCreatorRuntime")
            eventLog.appendEvent("creator_runtime_started", "phase3-runtime", selectedProfile)
            show(runtime?.nodeMetadata().orEmpty())
            refreshStatus("Runtime started")
        }.onFailure { showError(it) }
    }

    private fun stopRuntime() {
        runtime?.close()
        runtime = null
        stopService(Intent(this, CreatorForegroundService::class.java))
        eventLog.appendEvent("creator_runtime_stopped", "phase3-runtime", selectedProfile)
        refreshStatus("Runtime stopped")
    }

    private fun previewHostSeed() {
        runCatching {
            val payload = hostSeedInput.text.toString()
            val appPreview = HostSeedGuard.preview(payload)
            callRuntime("PreviewBootstrapDHTQR") { it.previewBootstrapDhtQr(payload) }
            lastHostSeedPayload = payload
            show(appPreview.toDisplay())
        }.onFailure { showError(it) }
    }

    private fun importHostSeed() {
        runCatching {
            val payload = hostSeedInput.text.toString()
            HostSeedGuard.preview(payload)
            callRuntime("ImportHostCreatorDHTSeed") { it.importHostCreatorDhtSeed(payload) }
            lastHostSeedPayload = payload
        }.onFailure { showError(it) }
    }

    private fun bootstrapNewCreator() {
        phase5PrerequisiteError(requireHostSeed = true)?.let {
            show("BootstrapNewCreator disabled: $it")
            return
        }
        callRuntime("BootstrapNewCreator") { runtime ->
            val chainId = nextChainId("bootstrap")
            runtime.bootstrapNewCreator("""{"chain_id":"$chainId"}""").also {
                lastBootstrapResult = it
                chainFilterInput.setText(chainId)
            }
        }
    }

    private fun sendDummy() {
        phase5PrerequisiteError(requireOnboarded = true)?.let {
            show("SendDummy disabled: $it")
            return
        }
        callRuntime("SendDummy") { runtime ->
            val chainId = nextChainId("dummy")
            runtime.sendDummy("""{"chain_id":"$chainId","size_bytes":256}""").also {
                chainFilterInput.setText(chainId)
            }
        }
    }

    private fun sendUpload() {
        phase5PrerequisiteError(requireOnboarded = true, requireUpload = true)?.let {
            show("SendUpload disabled: $it")
            return
        }
        callRuntime("SendUpload") { runtime ->
            val chainId = nextChainId("send-upload")
            val sessionId = jsonStringField(lastUploadResult, "session_id")
            val sessionField = sessionId?.let { ""","session_id":${JsonText.quote(it)}""" }.orEmpty()
            runtime.sendUpload("""{"chain_id":"$chainId"$sessionField,"target_lane_count":3}""").also {
                chainFilterInput.setText(chainId)
            }
        }
    }

    private fun exportEvidence(): EvidenceBundleResult {
        val chainId = nextChainId("evidence")
        val runtimeEvidence = runtime?.exportEvidence() ?: "{}"
        val files = linkedMapOf(
            "evidence.json" to """{"chain_id":"$chainId","runtime_export":${JsonText.quote(runtimeEvidence)}}""",
            "events.jsonl" to eventLog.readText(),
            "trace_events.jsonl" to (runtime?.traceEvents("{}") ?: "[]"),
            "local_dht.json" to (runtime?.localDht() ?: "{}"),
            "node_metadata.json" to (runtime?.nodeMetadata() ?: "{}"),
            "host_creator_seed.redacted.json" to (lastHostSeedPayload?.let { HostSeedGuard.redacted(it) } ?: "{}"),
            "upload_sessions.json" to lastUploadResult,
            "endpoint_config.redacted.json" to runProfile.rawJson,
            "device_context.json" to DeviceContext.deviceJson(this),
            "network_context.json" to DeviceContext.networkJson(this, "Phase 3 emulator-first validation"),
            "app_build.json" to DeviceContext.appBuildJson(),
            "rust_build.json" to DeviceContext.rustBuildJson(),
            "chain_ids.txt" to listOf(chainId, jsonStringField(lastBootstrapResult, "chain_id")).filterNotNull().joinToString("\n"),
            "remote_trace_queries.json" to runtimeTraceQueries(chainId),
        )
        val result = EvidenceBundleWriter.writeBundle(
            rootDir = File(cacheDir, "evidence-exports").apply { mkdirs() },
            bundleId = "mobile-evidence-${System.currentTimeMillis()}",
            files = files,
        )
        lastEvidence = result
        eventLog.appendEvent("creator_evidence_exported", chainId, result.zipFile.absolutePath)
        show("Evidence ZIP: ${result.zipFile.absolutePath}\nfiles=${result.fileCount}\nsha256=${result.zipSha256}")
        return result
    }

    private fun uploadEvidenceToS3() {
        runCatching {
            val evidence = lastEvidence ?: exportEvidence()
            val uploadConfig = EvidenceUploadConfig.parse(uploadGrantInput.text.toString())
            show("Uploading evidence to S3...\ns3://${uploadConfig.bucket}/${uploadConfig.objectKey}")
            Thread {
                runCatching {
                    S3EvidenceUploader.uploadPresignedPut(uploadConfig, evidence.zipFile)
                }.onSuccess { result ->
                    runOnUiThread {
                        show("Uploaded s3://${result.bucket}/${result.objectKey}\netag=${result.etag}\nsha256=${result.localSha256}")
                    }
                }.onFailure { error ->
                    runOnUiThread { showError(error) }
                }
            }.start()
        }.onFailure { showError(it) }
    }

    private fun importS3GrantFromDeviceFile() {
        runCatching {
            val grantFile = s3GrantFile()
            require(grantFile.exists()) {
                "S3 grant file missing. Push it with: adb push /tmp/pass4-s3-grant.json ${grantFile.absolutePath}"
            }
            val rawJson = grantFile.readText()
            val config = EvidenceUploadConfig.parse(rawJson)
            uploadGrantInput.setText(rawJson)
            show("Imported S3 evidence grant from ${grantFile.absolutePath}\nobject_key=${config.objectKey}\nexpires_at_ms=${config.expiresAtMs ?: 0}")
        }.onFailure { showError(it) }
    }

    private fun importS3GrantQrPayload(payload: String) {
        runCatching {
            val result = s3GrantQrAssembler.accept(payload)
            if (result.complete && result.grantJson != null) {
                uploadGrantInput.setText(result.grantJson)
            }
            show(result.message)
        }.onFailure { showError(it) }
    }

    private fun phase5PrerequisiteError(
        requireHostSeed: Boolean = false,
        requireOnboarded: Boolean = false,
        requireUpload: Boolean = false,
    ): String? {
        if (runtime == null) return "runtime is stopped"
        if (selectedProfile != RunProfileConfig.PROFILE_LOCAL_K8S_PUBLIC &&
            selectedProfile != RunProfileConfig.PROFILE_HYBRID
        ) {
            return "select local_k8s_public or hybrid profile"
        }
        if (requireHostSeed && lastHostSeedPayload.isNullOrBlank()) {
            return "import HostCreator DHT seed first"
        }
        if (requireOnboarded) {
            val dht = runCatching { runtime?.localDht().orEmpty() }.getOrDefault("")
            if (!dht.contains(""""self_onboarding_state":"onboarded"""") &&
                !dht.contains(""""self_onboarding_state":"fanout_partial"""")
            ) {
                return "mobile creator is not onboarded"
            }
        }
        if (requireUpload && jsonStringField(lastUploadResult, "session_id").isNullOrBlank()) {
            return "build an upload session first"
        }
        return null
    }

    private fun callRuntime(buttonId: String, block: (MobileCreatorRuntime) -> String) {
        val current = runtime
        if (current == null) {
            show("Runtime is stopped. Start runtime first.")
            return
        }
        runCatching {
            val action = CreatorActionCatalog.actions.firstOrNull { it.buttonId == buttonId || it.buttonId == "RefreshStatus" }
            if (action != null) eventLog.appendAction(action, nextChainId(buttonId.lowercase()), "running")
            show(block(current))
        }.onFailure { showError(it) }
    }

    private fun refreshStatus(message: String) {
        status.text = "$message\nprofile=$selectedProfile\nstate=${stateDir().absolutePath}"
    }

    private fun show(value: String) {
        output.text = value
        output.post { scrollView.smoothScrollTo(0, output.bottom) }
    }

    private fun showError(error: Throwable) {
        val message = error.message
            ?: error::class.java.simpleName.takeIf { it.isNotBlank() }
            ?: error.toString()
        output.text = "ERROR: $message"
        output.post { scrollView.smoothScrollTo(0, output.bottom) }
        eventLog.appendEvent("app_error", nextChainId("error"), message)
    }

    private fun jsonStringField(json: String, field: String): String? =
        Regex(""""${Regex.escape(field)}"\s*:\s*"([^"]*)"""")
            .find(json)
            ?.groupValues
            ?.get(1)

    private fun startForeground(chainId: String, operation: String) {
        val intent = Intent(this, CreatorForegroundService::class.java)
            .putExtra(CreatorForegroundService.EXTRA_CHAIN_ID, chainId)
            .putExtra(CreatorForegroundService.EXTRA_OPERATION, operation)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }

    private fun requestPhase3Permissions() {
        if (Build.VERSION.SDK_INT >= 23) {
            val permissions = buildList {
                add(Manifest.permission.CAMERA)
                if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
            if (permissions.isNotEmpty()) requestPermissions(permissions.toTypedArray(), 13013)
        }
    }

    private fun runtimeTraceQueries(chainId: String): String =
        """
        [
          {"chain_id":"$chainId","surface":"local_k8s_publisher_authority","query_hint":"kubectl logs deploy/publisher-authority -n veritas | grep $chainId"},
          {"chain_id":"$chainId","surface":"local_k8s_publisher_receiver","query_hint":"kubectl logs deploy/publisher-receiver -n veritas | grep $chainId"},
          {"chain_id":"$chainId","surface":"local_k8s_exitbridges","query_hint":"kubectl logs statefulset/exit-bridge -n veritas --all-containers | grep $chainId"},
          {"chain_id":"$chainId","surface":"aws_exitbridge_cloudwatch","region":"ca-central-1","query_hint":"aws logs filter-log-events --region ca-central-1 --filter-pattern $chainId"}
        ]
        """.trimIndent()

    private fun sampleHostSeedPayload(): String {
        val expires = System.currentTimeMillis() + 86_400_000
        return """
        {
          "schema_version": 1,
          "chain_id": "pass4-phase3-host-seed",
          "run_id": "pass4-phase3-emulator",
          "host_creator_id": "host-creator",
          "host_creator_public_key_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "host_creator_entry": {
            "node_id": "host-creator",
            "ip_addr": "198.51.100.10",
            "pub_key": [170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170,170],
            "udp_punch_port": 4443,
            "entry_expiry_ms": $expires,
            "publisher_sig": [1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],
            "active": true
          },
          "host_creator_reachability": {
            "reachability_class": "direct",
            "capabilities": ["bootstrap_seed"]
          },
          "host_creator_bootstrap_endpoints": [
            {
              "protocol": "https",
              "host": "host-creator.example.test",
              "port": 443,
              "tls_sni": "host-creator.example.test"
            }
          ],
          "issued_at_ms": ${System.currentTimeMillis()},
          "expires_at_ms": $expires,
          "payload_hash": "sha256:phase3-sample",
          "signature": "phase3-sample-signature"
        }
        """.trimIndent()
    }

    private fun stateDir(): File = File(filesDir, "creator-runtime")

    private fun evidenceDir(): File = File(stateDir(), "evidence")

    private fun s3GrantFile(): File =
        File(getExternalFilesDir(null) ?: filesDir, "pass4-s3-grant.json")

    private fun deviceId(): String =
        Settings.Secure.getString(contentResolver, Settings.Secure.ANDROID_ID) ?: "emulator"

    private fun nextChainId(prefix: String): String = "phase3-$prefix-${System.currentTimeMillis()}"

    private fun findAction(buttonId: String): CreatorAction =
        CreatorActionCatalog.actions.first { it.buttonId == buttonId }

    private fun title(text: String): TextView =
        TextView(this).apply {
            this.text = text
            textSize = 24f
            setTypeface(typeface, Typeface.BOLD)
        }

    private fun section(text: String): TextView =
        TextView(this).apply {
            this.text = "\n$text"
            textSize = 18f
            setTypeface(typeface, Typeface.BOLD)
            setTag("Screen-$text")
        }

    private fun body(text: String): TextView =
        TextView(this).apply {
            this.text = text
            textSize = 14f
            setPadding(0, 8, 0, 8)
        }

    private fun button(id: String, label: String, onClick: () -> Unit): Button =
        Button(this).apply {
            text = label
            contentDescription = id
            setTag(id)
            setOnClickListener { onClick() }
        }

    private fun disabledButton(id: String, label: String, reason: String): Button =
        Button(this).apply {
            text = "$label disabled: $reason"
            contentDescription = id
            setTag(id)
            isEnabled = false
        }

    private fun multiLine(text: String, tag: String): EditText =
        EditText(this).apply {
            setText(text)
            minLines = 4
            maxLines = 12
            setHorizontallyScrolling(false)
            setTag(tag)
        }

    companion object {
        private const val REQUEST_HOST_QR_SCAN = 13014
        private const val REQUEST_S3_GRANT_QR_SCAN = 13015
    }
}
