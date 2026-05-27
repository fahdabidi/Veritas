package com.veritas.gbn.mobile

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.view.Gravity
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ExperimentalGetImage
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import com.google.mlkit.vision.barcode.BarcodeScanner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.Executor
import java.util.concurrent.Executors

@androidx.annotation.OptIn(markerClass = [ExperimentalGetImage::class])
class QrScannerActivity : ComponentActivity() {
    private lateinit var previewView: PreviewView
    private lateinit var status: TextView

    private val mainExecutor = Executor { command -> runOnUiThread(command) }
    private val analysisExecutor = Executors.newSingleThreadExecutor()
    private val scanner: BarcodeScanner = BarcodeScanning.getClient(
        BarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .build(),
    )
    private val cameraPermissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) {
            startCamera()
        } else {
            status.text = "Camera permission denied. Use document import as the fallback."
        }
    }

    private var cameraProvider: ProcessCameraProvider? = null
    private var completing = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(buildUi())
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            startCamera()
        } else {
            cameraPermissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }

    override fun onDestroy() {
        cameraProvider?.unbindAll()
        scanner.close()
        analysisExecutor.shutdown()
        super.onDestroy()
    }

    private fun buildUi(): LinearLayout {
        val scannerTitle = intent.getStringExtra(EXTRA_SCANNER_TITLE) ?: "QR Scanner"
        previewView = PreviewView(this).apply {
            layoutParams = LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                0,
                1f,
            )
            scaleType = PreviewView.ScaleType.FILL_CENTER
        }
        status = TextView(this).apply {
            text = "Point the camera at $scannerTitle"
            textSize = 16f
            gravity = Gravity.CENTER
            setPadding(24, 24, 24, 24)
        }
        return LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(status)
            addView(previewView)
            addView(Button(this@QrScannerActivity).apply {
                text = "Cancel"
                setOnClickListener { finish() }
            })
        }
    }

    private fun startCamera() {
        val providerFuture = ProcessCameraProvider.getInstance(this)
        providerFuture.addListener(
            {
                runCatching {
                    val provider = providerFuture.get()
                    cameraProvider = provider
                    val preview = Preview.Builder()
                        .build()
                        .also { it.setSurfaceProvider(previewView.surfaceProvider) }
                    val analysis = ImageAnalysis.Builder()
                        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                        .build()
                        .also { it.setAnalyzer(analysisExecutor, ::analyzeQrFrame) }

                    provider.unbindAll()
                    provider.bindToLifecycle(
                        this,
                        CameraSelector.DEFAULT_BACK_CAMERA,
                        preview,
                        analysis,
                    )
                    status.text = "Camera ready. Align the QR code in view."
                }.onFailure { error ->
                    status.text = "Camera failed: ${error.message ?: error::class.java.simpleName}"
                }
            },
            mainExecutor,
        )
    }

    private fun analyzeQrFrame(imageProxy: ImageProxy) {
        if (completing) {
            imageProxy.close()
            return
        }
        val mediaImage = imageProxy.image
        if (mediaImage == null) {
            imageProxy.close()
            return
        }
        val image = InputImage.fromMediaImage(mediaImage, imageProxy.imageInfo.rotationDegrees)
        scanner.process(image)
            .addOnSuccessListener { barcodes ->
                val payload = barcodes.firstNotNullOfOrNull { barcode ->
                    barcode.rawValue?.takeIf { it.isNotBlank() }
                }
                if (payload != null) complete(payload)
            }
            .addOnFailureListener { error ->
                status.post { status.text = "QR scan failed: ${error.message ?: error::class.java.simpleName}" }
            }
            .addOnCompleteListener {
                imageProxy.close()
            }
    }

    private fun complete(payload: String) {
        if (completing) return
        completing = true
        runOnUiThread {
            setResult(RESULT_OK, Intent().putExtra(EXTRA_QR_PAYLOAD, payload))
            finish()
        }
    }

    companion object {
        const val EXTRA_SCANNER_TITLE = "com.veritas.gbn.mobile.EXTRA_SCANNER_TITLE"
        const val EXTRA_QR_PAYLOAD = "com.veritas.gbn.mobile.EXTRA_QR_PAYLOAD"
    }
}
