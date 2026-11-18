import { useQuery } from "@tanstack/react-query";
import { Canvas, useFrame } from "@react-three/fiber/native";
import { Asset } from "expo-asset";
import { File } from "expo-file-system";
import { Box3, Group, Mesh, MeshStandardMaterial, Vector3 } from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";

const modelAsset = require("../assets/models/mac-mini.glb") as number;
const ledMaterialName = "white_led";
const shellMaterialName = "base_metal";
const shellColor = "#d3d6db";
const modelSize = 2.2;

type Model = { scene: Group; led: MeshStandardMaterial | undefined; facing: number };

async function loadModel(): Promise<Model> {
  const [asset] = await Asset.loadAsync(modelAsset);
  if (!asset?.localUri) {
    throw new Error("Model not found");
  }
  const data = await new File(asset.localUri).arrayBuffer();
  const gltf = await new GLTFLoader().parseAsync(data, "");
  const scene = gltf.scene;
  scene.updateMatrixWorld(true);

  const box = new Box3().setFromObject(scene);
  const center = box.getCenter(new Vector3());
  const scale = modelSize / Math.max(...box.getSize(new Vector3()).toArray());

  let led: MeshStandardMaterial | undefined;
  let ledCenter: Vector3 | undefined;
  scene.traverse((object) => {
    if (!(object instanceof Mesh)) {
      return;
    }
    if (object.material.name === shellMaterialName) {
      // No environment map in the scene, so a metallic material would render black.
      object.material.color.set(shellColor);
      object.material.metalness = 0.1;
      object.material.roughness = 0.45;
    }
    if (object.material.name !== ledMaterialName) {
      return;
    }
    led = object.material.clone() as MeshStandardMaterial;
    object.material = led;
    ledCenter = new Box3().setFromObject(object).getCenter(new Vector3());
  });

  // The LED sits off-centre on its face, so snap to the nearest side (multiple of 90°)
  // to turn that whole face squarely towards the camera (+z).
  const quarterTurn = Math.PI / 2;
  const facing = ledCenter
    ? Math.round(Math.atan2(ledCenter.x - center.x, ledCenter.z - center.z) / quarterTurn) *
      quarterTurn
    : 0;

  scene.position.set(-center.x * scale, -center.y * scale, -center.z * scale);
  scene.scale.setScalar(scale);
  const pivot = new Group();
  pivot.add(scene);
  return { scene: pivot, led, facing };
}

export type LedState = "off" | "on" | "pulsing";

function setLed({ led }: Model, state: LedState, time: number) {
  if (!led) {
    return;
  }
  let level = 0;
  if (state === "pulsing") {
    level = 0.5 + 0.5 * Math.sin(time * 5);
  } else if (state === "on") {
    level = 1;
  }
  led.emissive.set(level > 0 ? "#ffffff" : "#000000");
  led.emissiveIntensity = level * 3;
  led.color.set(level > 0 ? "#ffffff" : "#222222");
}

function Mini({ model, led }: { model: Model; led: LedState }) {
  useFrame(({ clock }) => {
    setLed(model, led, clock.elapsedTime);
  });

  return (
    <group rotation={[0.35, -model.facing, 0]}>
      <primitive object={model.scene} />
    </group>
  );
}

export function MacMini({ led }: { led: LedState }) {
  const { data: model } = useQuery({
    queryKey: ["mac-mini-model"],
    queryFn: loadModel,
    staleTime: Infinity,
    gcTime: Infinity,
  });

  return (
    <Canvas flat camera={{ position: [0, 1.6, 5], fov: 35 }}>
      <hemisphereLight args={["#ffffff", "#8899aa", 1.6]} />
      <directionalLight position={[3, 5, 4]} intensity={2.5} />
      <directionalLight position={[-4, 3, 3]} intensity={1.2} />
      {model && <Mini model={model} led={led} />}
    </Canvas>
  );
}
