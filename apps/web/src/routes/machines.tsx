import { createFileRoute } from "@tanstack/react-router";
import { MachinesPage } from "../components/MachinesPage";

export const Route = createFileRoute("/machines")({ component: MachinesPage });
